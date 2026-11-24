//! What script work costs in Studio: the seconds os.clock advances by while a script calls the
//! engine (the work clock, scheduler.rs). Budgeted work (a loop that yields once os.clock moved
//! a few ms since its slice began) then takes as many frames as in Studio instead of one.
//!
//! Measured with os.clock over 200-20,000 calls (two runs each, the mean taken) in a playtest's
//! client of a game's effect scene ("playtest") and in an edit-mode capture
//! Studio ("capture"; its Explorer makes instance moves slower than a running game's), then
//! checked against that playtest's own preparation: its 3,085 grass copies (33 us median a copy,
//! 0.138 s in all) and 952 mesh slices (docs/engine.md "Work clock"). Scripts' own Luau is
//! charged from the VM's work counters ([`luau`]); the busy-loop allowance
//! (Scheduler.clock_call_allowance) still ends a loop that only reads the clock.

use crate::{ClassId, Reflection};

use std::{
    cell::RefCell,
    collections::BTreeMap,
    sync::atomic::{AtomicBool, Ordering},
};

static TALLY_ON: AtomicBool = AtomicBool::new(false);

thread_local! {
    static TALLY: RefCell<BTreeMap<String, (u64, f64)>> = const { RefCell::new(BTreeMap::new()) };
}

/// `RUR_CHARGE_TALLY=1` counts every engine charge of the run (how many times, how many seconds), by the amount
/// charged and, for a property write, by class, property and whether it changed or was placed: `dump_tally` prints
/// it to stderr when the process ends. Off, a charge pays one relaxed load.
pub fn enable_tally() {
    TALLY_ON.store(true, Ordering::Relaxed);
}

#[inline]
pub fn tally_charge(seconds: f64) {
    if TALLY_ON.load(Ordering::Relaxed) {
        tally(format!("charge {:9.1} ns", seconds * 1e9), seconds);
    }
}

pub fn tally_write(class: &str, property: &str, changed: bool, placed: bool, seconds: f64) {
    if TALLY_ON.load(Ordering::Relaxed) {
        tally(format!("write {class}.{property} changed={changed} placed={placed}"), seconds);
    }
}

pub fn tally_member(kind: &str, class: &str, member: impl FnOnce() -> String) {
    if TALLY_ON.load(Ordering::Relaxed) {
        tally(format!("{kind} {class}.{}", member()), 0.0);
    }
}

fn tally(key: String, seconds: f64) {
    TALLY.with(|table| {
        let mut table = table.borrow_mut();
        let entry = table.entry(key).or_insert((0, 0.0));
        entry.0 += 1;
        entry.1 += seconds;
    });
}

pub fn dump_tally() {
    if !TALLY_ON.load(Ordering::Relaxed) {
        return;
    }
    TALLY.with(|table| {
        for (key, (count, seconds)) in table.borrow().iter() {
            eprintln!("TALLY\t{key}\t{count}\t{:.3}", seconds * 1e6);
        }
    });
}

/// The first touch of an instance in a frame (a read, a write or a method call) costs more than the next ones: the frame's render,
/// animation and physics work has pushed the instance out of the CPU's caches, so a script that touches many instances once a
/// frame pays a miss on each, and a loop that hammers one instance (every op_cost row) never does (a bridge_ops probe:
/// 28 R15 rigs in an empty test place's client, a handler bracketed with os.clock, the median of 210 frames less the empty
/// handler, per operation: a read of Part.CFrame touching 5 parts per rig once 322 ns, twice each 205, eight times each 80; Part.Size
/// 281 / - / 63; IsA 313 / - / 72; a Motor6D.Transform write 344 / - / 92; three properties of one part, three parts, twice 99:
/// the miss belongs to the instance, not the property). That probe ran in a Studio playtest client on animated rigs; the parity bench
/// (tools/cost-parity) reads one static part of 100 once each a frame at 74.6 ns where a hot read of the same property is 29.4, in a
/// rendered client of a game's match place (roblox-sim, Studio 0.744): a static instance's first touch adds 45 ns. A cold geometry read
/// is dearer (the bench's CFrame of 100 parts once each, 175 ns, a row held out of the fit; the rig probe's 322): not modelled apart.
pub const COLD_TOUCH: f64 = 0.045e-6;

/// The first touch in a frame of a GUI object or a UI component ([`COLD_TOUCH`] is a part's): a client lays out and draws every GUI
/// object every frame, so a script that walks a GUI finds each one out of its caches. Measured in a rendered client of a game's lobby
/// (roblox-sim, Studio 0.744; a grid of 300 cards laid out in a ScrollingFrame, one loop in a frame of its own each, the median of 15
/// frames): AbsolutePosition of 300 distinct cards 273 ns a read against 71 for one card read 300 times, a UIGradient's Color read on
/// 90 distinct gradients 416 against a hot read's 77; Color written on 90 distinct gradients 754 against one gradient's 425, Offset 539
/// against 178, a Frame's BackgroundColor3 698 against 318. The first touch is 270 ns (the reads' 202 and 339), a write's dirtying
/// [`cold_write_extra`] on top.
pub const COLD_TOUCH_GUI: f64 = 0.27e-6;

/// Which first-touch price an instance of `class` pays ([`Touched::touch`]): a GUI object's or UI component's ([`COLD_TOUCH_GUI`]),
/// else [`COLD_TOUCH`] (one made right after or right before the instance touched cold before it: [`COLD_TOUCH_ALONE`] when not).
pub fn cold_touch(reflection: &Reflection, class: ClassId) -> f64 {
    if ColdKind::of(reflection, class) == ColdKind::Gui { COLD_TOUCH_GUI } else { COLD_TOUCH }
}

/// The first touch in a frame of an instance touched by itself: one not made right after (or right before) the instance touched cold
/// before it. [`COLD_TOUCH`] is what a client pays for instances made together, one after another, and touched in that order (the
/// hardware fetches the next one ahead); an effect's instances, cloned over many frames between other work and touched among other
/// code's, are each a miss of their own. Measured in rendered and headless clients of a game's match place (roblox-sim, Studio 0.744;
/// notes/macro castprobe, four rendered sessions, the median of 15 frames): 48 parts, each from a clone made a frame apart with other
/// clones and allocations between, touched once each in a frame (after a first touch of the kind, so the member's code is warm),
/// against 48 parts made together in one loop: GetAttribute 0.52 us against 0.10, FindFirstChild 0.78 / 0.11, IsA 0.30 / 0.10, a Size
/// read 0.40 / 0.11, a Transparency write 0.40 / 0.13, a CFrame write 0.78 / 0.41, an emitter's TimeScale read 0.35 (hot 0.04): 0.29 us
/// more at the median. The engine has no view of where an instance lies, so "made together" is creation order: the instance's serial
/// next to the last cold one's. An instance a script touches every frame pays the plain first touch whatever its order: an effect
/// system's step lists reading every emitter of four waves of effects once a frame read 0.85-1.17x a client with it (notes/macro vfx)
/// and 1.2-2.0x with these touches priced alone; the probe's lone instances were last touched two frames before.
pub const COLD_TOUCH_ALONE: f64 = 0.32e-6;

/// The engine paths whose code a client runs cold at their first use in a frame, beyond a member's first use ([`FIRST_CALL`] and the
/// rest): a frame runs render, physics and animation work between two frames' scripts, so the first clone, destroy, move or emit of a
/// frame runs the engine's code for it, and for each class it handles, out of the CPU's caches. Measured in rendered clients of a
/// game's match place (roblox-sim, Studio 0.744; notes/macro castprobe, four sessions, each case in a frame of its own after a few
/// engine calls every frame makes anyway, the median of 15 rounds), as the frame's first such call less a second one on another cold
/// copy, less what the member's first use already costs here: a clone 25 us and each class it holds that the frame has not cloned yet
/// ([`first_cloned`]: a part 55 us in all, a part with 4 emitters 96, a model of 5 parts each with an attachment and 2 emitters 128,
/// a folder of 10 values 40); a destroy 10 us and its classes ([`first_destroyed`]: a part 47, a folder of 10 values 22); entering the
/// Workspace 13.5 us (a folder 13.5, anything with a part 25-31: [`ENTER_PART_FIRST`]); a PivotTo 17-23, a part's CFrame write 10-12,
/// BulkMoveTo 25-30, Emit 5.2, QueryDescendants 7.4, GetDescendants 4.8 (with its call), an emitter's Enabled write 3.8. Headless
/// clients read the same within 20 % (a frame's first entry 21 for a part). Once a path ran in a frame, a second use costs only what
/// a cold instance adds ([`clone_cold`], [`PIVOT_COLD_PART`] and the rest) over the hot prices.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Path {
    Clone = 0,
    Destroy = 1,
    Enter = 2,
    EnterPart = 3,
    Pivot = 4,
    BulkMove = 5,
    Geometry = 6,
    Emit = 7,
    Query = 8,
    Descendants = 9,
    Toggle = 10,
}
const PATHS: usize = 11;
pub const PATH_FIRST: [f64; PATHS] = [25.0e-6, 10.0e-6, 13.5e-6, ENTER_PART_FIRST, 16.0e-6, 28.0e-6, 9.5e-6, 4.4e-6, 6.6e-6, 3.6e-6, 2.4e-6];
/// What the first part entering the Workspace in a frame adds to [`Path::Enter`]'s (a part 26 us, a folder 13.5)
pub const ENTER_PART_FIRST: f64 = 12.5e-6;

/// What the frame's first clone of an instance of `class` adds beyond [`Path::Clone`] (castprobe, each template's first clone in a
/// frame less its twin's: a part 30 us, a MeshPart 55, a model 32, an attachment 12, a particle emitter 35, a beam 11, a trail 10, a
/// light 12, a decal 27, a mesh 17, a folder or a value 7; a class the probe has no template of 10). Once a class was cloned in a
/// frame, more of it cost what the clone prices say.
pub fn first_cloned(reflection: &Reflection, class: ClassId) -> f64 {
    1e-6 * match &*reflection.class(class).name {
        "MeshPart" => 55.0,
        "Model" => 32.0,
        "Attachment" => 12.0,
        "ParticleEmitter" => 35.0,
        "Beam" => 11.0,
        "Trail" => 10.0,
        "Decal" | "Texture" => 27.0,
        "Folder" => 7.0,
        _ if reflection.is_a(class, "BasePart") => 30.0,
        _ if reflection.is_a(class, "Light") => 12.0,
        _ if reflection.is_a(class, "DataModelMesh") => 17.0,
        _ if reflection.is_a(class, "ValueBase") => 7.0,
        _ => 10.0,
    }
}

/// What the frame's first destroy of an instance of `class` adds beyond [`Path::Destroy`] (castprobe, the frame's first destroy of each
/// template's copy less the second: a part 36 us, a MeshPart 40, a model 15, an attachment 11, a particle emitter 11, a beam 1, a trail
/// 4, a light 14, a decal 6.5, a mesh 6.5, a folder or a value 5.5; another class 6).
pub fn first_destroyed(reflection: &Reflection, class: ClassId) -> f64 {
    1e-6 * match &*reflection.class(class).name {
        "MeshPart" => 40.0,
        "Model" => 15.0,
        "Attachment" => 11.0,
        "ParticleEmitter" => 11.0,
        "Beam" => 1.0,
        "Trail" => 4.0,
        "Decal" | "Texture" => 6.5,
        "Folder" => 5.5,
        _ if reflection.is_a(class, "BasePart") => 36.0,
        _ if reflection.is_a(class, "Light") => 14.0,
        _ if reflection.is_a(class, "DataModelMesh") => 6.5,
        _ if reflection.is_a(class, "ValueBase") => 5.5,
        _ => 6.0,
    }
}

/// What cloning an instance a frame has not touched adds to [`cloned`]: the copy reads every property of a template no code has
/// touched since an earlier frame (castprobe: a twin template's clone less the same template cloned again, per instance it holds: a
/// part 1.8 us, a MeshPart 4.0, a particle emitter 1.8, a beam 1.5, a trail 1.7, a decal 1.9, a mesh 0.9, a light 0.5, an
/// attachment 0.1, a folder or a value 0.4).
pub fn clone_cold(reflection: &Reflection, class: ClassId) -> f64 {
    1e-6 * match &*reflection.class(class).name {
        "MeshPart" => 4.0,
        "ParticleEmitter" => 1.8,
        "Beam" => 1.5,
        "Trail" => 1.7,
        "Decal" | "Texture" => 1.9,
        "Attachment" => 0.1,
        _ if reflection.is_a(class, "BasePart") => 1.8,
        _ if reflection.is_a(class, "DataModelMesh") => 0.9,
        _ if reflection.is_a(class, "Light") => 0.5,
        _ => 0.4,
    }
}

/// What destroying an instance that has been in the Workspace since an earlier frame adds to [`DESTROYED`] and its leaving: the
/// engine took it into its render, physics and particle work, which a destroy undoes (castprobe: a template's copy parented two
/// frames before, destroyed third in a frame, against ree's price for the same: a part 6.65 us in all against 2.28, so 4.4 more, a
/// MeshPart 4.5, an attachment 1.4, a particle emitter 2.1, a beam 0.7, a trail 1.0, a light 2.1, a decal 1.6, a mesh 0.8, a model 1.3,
/// a folder or a value 0.5). The parity corpus destroys what it made in the same frame, which pays none of this.
pub fn destroy_live(reflection: &Reflection, class: ClassId) -> f64 {
    1e-6 * match &*reflection.class(class).name {
        "MeshPart" => 4.5,
        "Model" => 1.3,
        "Attachment" => 1.4,
        "ParticleEmitter" => 2.1,
        "Beam" => 0.7,
        "Trail" => 1.0,
        "Decal" | "Texture" => 1.6,
        _ if reflection.is_a(class, "BasePart") => 4.4,
        _ if reflection.is_a(class, "Light") => 2.1,
        _ if reflection.is_a(class, "DataModelMesh") => 0.8,
        _ => 0.5,
    }
}

/// What moving a part no code touched in this frame or the one before adds, by PivotTo (castprobe: a second PivotTo on another kept
/// copy, untouched for about 20 frames, less the first again: a part 0.9-1.4 us, a part with 4 emitters 3.35, a model of 5 parts 7.0,
/// of 5 parts with 15 attachments and emitters 16.4) and by BulkMoveTo (a second call's 8 parts less the first's again: 1.29 us a part,
/// 2.41 with 4 emitters): a part, and each emitter under it (PivotTo every attachment or effect under it). A part a script moves every
/// frame pays none: a game's attack cast that moves 35 rocks a frame by BulkMoveTo, timed in a client, pays about 1.4 us a part-move in all.
pub const PIVOT_COLD_PART: f64 = 1.2e-6;
pub const PIVOT_COLD_ATTACHED: f64 = 0.6e-6;
pub const BULK_COLD_PART: f64 = 1.25e-6;
pub const BULK_COLD_EMITTER: f64 = 0.28e-6;
/// What an emit from an emitter the frame has not touched adds (castprobe: a second Emit(4) on another emitter less the first again,
/// 1.7 us against ree's touch), what switching one on adds (0.9 against 0.2), and what a query from a root the frame has not touched
/// adds (QueryDescendants of a model of 21 instances two frames old, the second query less the first again, 5.0 us). That one is
/// priced by the call, not by the nodes: a walk of a whole service's static instances costs a client less a node than a fresh
/// model's, and ree's services can hold far more than a client's place (a game's mounted asset tree under ReplicatedStorage).
pub const EMIT_COLD: f64 = 1.45e-6;
pub const TOGGLE_COLD: f64 = 0.7e-6;
pub const QUERY_COLD: f64 = 4.6e-6;

/// What a changed write adds to the first touch of an instance in a frame beyond [`COLD_TOUCH`]. A write dirties more of the
/// instance than a read touches (the property's slot, its change flags, the render or physics structure that hears of it), so a roster
/// of 100 distinct instances written once each costs more than 100 times the loop's price plus a read's miss: a cold_write probe in
/// a match place's client, the medians of its runs, per write fitted from the totals at 100 and 300 instances, less the same writes on
/// one instance: Transparency +388 ns, Color +357, CFrame +773, Name +559, SetAttribute +830,
/// against ree's 280 (a read's miss). This table is the rest, so the roster of a HUD or a rig lands within 1.3x of Studio's per-write
/// slope. A frame's first writes pay a fixed cost on top (a first_write probe: the first changed write of a frame 14.1 us, a property
/// written for the first time in it 4-18 us more, then 1-2 us for the next instance and 0.3 once a loop runs; it is cold code after the
/// render and physics steps): [`FIRST_WRITE`] and its kin charge a member's first use in a frame, and the first touches of instances
/// and engine paths theirs. Those charges belong to the frame's first user, not to a cut of it: each is booked as a cold charge to the
/// object it is for (the member, the instance's class, the engine path) with who paid it and whether other code touched it that frame
/// ([`Touched`], src/profiler/attribution.rs), so a profile's saving for a cut (`saving_ms`) counts it only when nothing else used it.
///
/// A GuiObject (`part` false; the same bench's `Frame` rows, five runs: Size +475..525, Position +600, BackgroundTransparency +380 over
/// the hot write, noisier than the Parts': Position's n100 readings span 781-1550 ns) is dearer on Size and Position and close on the
/// transparencies.
///
/// With the first touch at 45 ns ([`COLD_TOUCH`]) each extra below is the cold_write probe's total for the property less that, so the
/// totals stay what it measured; the parity bench's rosters agree where they were measured (100 parts' CFrame 731 ns over the hot write,
/// 100 frames' BackgroundTransparency 356: rows held out of the fit), but for a part's Transparency, which it reads at 143 (fitted: 98).
/// A GUI object's first touch is [`COLD_TOUCH_GUI`]'s 270 ns, so its extras are the same totals less that: Size and Position 310, the
/// transparencies 110, and any other property 75 (the lobby grid's writes on distinct objects above: Color, Offset and
/// BackgroundColor3 329-380 ns over a hot write).
pub fn cold_write_extra(kind: ColdKind, property: &str) -> f64 {
    1e-9 * match (kind, property) {
        (ColdKind::Part, "CFrame" | "Position" | "Orientation" | "Rotation" | "PivotOffset" | "Size") => 735.0,
        (ColdKind::Part, "Color" | "BrickColor" | "Material") => 315.0,
        (ColdKind::Part, "Transparency" | "LocalTransparencyModifier") => 98.0,
        (ColdKind::Gui, "Size" | "Position") => 310.0,
        (ColdKind::Gui, "BackgroundTransparency" | "TextTransparency" | "ImageTransparency") => 110.0,
        (ColdKind::Gui, "Name") => 290.0,
        (ColdKind::Gui, "SetAttribute") => 560.0,
        (ColdKind::Gui, _) => 75.0,
        (_, "Name") => 515.0,
        (_, "SetAttribute") => 785.0,
        // (a joint's or bone's Transform and the properties not measured: the first touch alone, as fitted on Motor6D.Transform)
        _ => 0.0,
    }
}

/// What an instance's cold costs are priced as ([`cold_touch`], [`cold_write_extra`]): a part, a GUI object or UI component, or any
/// other instance.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ColdKind {
    Part,
    Gui,
    Other,
}

impl ColdKind {
    pub fn of(reflection: &Reflection, class: ClassId) -> Self {
        if reflection.is_a(class, "BasePart") {
            ColdKind::Part
        } else if reflection.is_a(class, "GuiBase2d") || reflection.is_a(class, "UIComponent") {
            ColdKind::Gui
        } else {
            ColdKind::Other
        }
    }
}

/// What a script uses of an instance's members, for the first use of each in a frame ([`Touched::first_use`])
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum MemberUse {
    Read = 0,
    Write = 1,
    Call = 2,
}

/// The key of a read that finds a child by its name rather than a member (the engine's own lookup code, whatever the name)
pub const CHILD_LOOKUP: crate::reflection::NameId = crate::reflection::NameId::MAX;

/// The first use in a frame of each of an instance's members a script reads, writes or calls, by kind: the engine's code for it
/// runs cold after the frame's render and physics steps (a rendered client of a game's lobby, roblox-sim, Studio 0.744: one call at
/// a time right after Heartbeat resumed the probe, each on an instance of its own, the median of 42 frames, against the same
/// sequence run again in the frame on other instances; notes/macro firstprobe): a frame's first GUI property write 1.65 us more,
/// the first write of another property 1.1-1.45, a part's first writes 0.9 (Transparency) and 2.35 (CFrame), a child found by name
/// and written 2.4-2.6; the first rect read 1.5 and a plain property's 0.3; a method's first call 0.6-0.95 (FindFirstChild,
/// GetAttribute, GetPivot). A member's second use in the frame costs 0.2-0.3 more than hot and later ones 0.08: not charged.
pub const FIRST_READ: f64 = 0.5e-6;
pub const FIRST_WRITE: f64 = 1.4e-6;
pub const FIRST_CALL: f64 = 0.8e-6;

/// What a run's scripts asked the engine to do, counted where the work clock charges it ([`Touched::ops`]): the engine's side of
/// a cast, whatever the work clock prices it at. A profile's marks carry the counts so far (profiler::Mark::engine), so a
/// phase's are the difference of two marks'.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EngineOps {
    /// Instances made: each instance of a Clone's copy, and each Instance.new
    pub cloned: u64,
    pub created: u64,
    /// Instances destroyed (each of a Destroy's subtree), and instances that entered the Workspace (each of the subtree a
    /// parent change took into it)
    pub destroyed: u64,
    pub entered: u64,
    /// Scripts' property writes; those that changed the value; of those, a part's in the Workspace that moved or resized it
    /// (CFrame, Position, Orientation, Rotation, Size), and an emitter's, beam's or trail's Enabled
    pub writes: u64,
    pub changed: u64,
    pub geometry: u64,
    pub toggles: u64,
    /// PivotTo on what is in the Workspace, and the parts BulkMoveTo moved
    pub pivots: u64,
    pub bulk_moved: u64,
    /// ParticleEmitter:Emit calls, and descendant queries (GetDescendants, QueryDescendants)
    pub emits: u64,
    pub queries: u64,
}

impl EngineOps {
    /// Each count by name, in the order profile.json and the summary give them
    pub fn named(&self) -> [(&'static str, u64); 12] {
        [
            ("cloned", self.cloned),
            ("created", self.created),
            ("destroyed", self.destroyed),
            ("entered", self.entered),
            ("writes", self.writes),
            ("changed", self.changed),
            ("geometry", self.geometry),
            ("toggles", self.toggles),
            ("pivots", self.pivots),
            ("bulk_moved", self.bulk_moved),
            ("emits", self.emits),
            ("queries", self.queries),
        ]
    }

    /// The counts from `earlier` to these (a phase's, from its mark to the next)
    pub fn since(&self, earlier: &EngineOps) -> EngineOps {
        EngineOps {
            cloned: self.cloned.saturating_sub(earlier.cloned),
            created: self.created.saturating_sub(earlier.created),
            destroyed: self.destroyed.saturating_sub(earlier.destroyed),
            entered: self.entered.saturating_sub(earlier.entered),
            writes: self.writes.saturating_sub(earlier.writes),
            changed: self.changed.saturating_sub(earlier.changed),
            geometry: self.geometry.saturating_sub(earlier.geometry),
            toggles: self.toggles.saturating_sub(earlier.toggles),
            pivots: self.pivots.saturating_sub(earlier.pivots),
            bulk_moved: self.bulk_moved.saturating_sub(earlier.bulk_moved),
            emits: self.emits.saturating_sub(earlier.emits),
            queries: self.queries.saturating_sub(earlier.queries),
        }
    }

    /// These counts and `other`'s
    pub fn plus(&self, other: &EngineOps) -> EngineOps {
        EngineOps {
            cloned: self.cloned + other.cloned,
            created: self.created + other.created,
            destroyed: self.destroyed + other.destroyed,
            entered: self.entered + other.entered,
            writes: self.writes + other.writes,
            changed: self.changed + other.changed,
            geometry: self.geometry + other.geometry,
            toggles: self.toggles + other.toggles,
            pivots: self.pivots + other.pivots,
            bulk_moved: self.bulk_moved + other.bulk_moved,
            emits: self.emits + other.emits,
            queries: self.queries + other.queries,
        }
    }
}

/// The kinds of work a profile keeps apart (profiler/vm.rs SPLITS), as bits of a mask: what a charge is (a cold or first-use charge,
/// the collector's, the engine's dispatch of a handler or resumption of a thread) and where its price came from (a Studio-era
/// measurement, or a price borrowed from another call: defaulted). A price with neither of the last two was measured in a client.
pub mod kind {
    pub const COLD: u8 = 1 << 0;
    pub const COLLECTOR: u8 = 1 << 1;
    pub const STUDIO: u8 = 1 << 2;
    pub const DEFAULTED: u8 = 1 << 3;
    pub const DISPATCH: u8 = 1 << 4;
}

/// Running totals of the engine's charges of each kind ([`kind`]; profiler/vm.rs SPLITS cells), beside the work clock's own total
/// (Scheduler::work and work_drained): the profile books what they moved with each instruction's work. One per engine, shared like
/// the work cell.
#[repr(C)]
#[derive(Default)]
pub struct WorkKinds {
    cells: [std::cell::Cell<f64>; 5],
}

impl WorkKinds {
    /// Adds `seconds` to each kind `mask` names
    #[inline]
    pub fn add(&self, mask: u8, seconds: f64) {
        if mask == 0 {
            return;
        }
        for (bit, cell) in self.cells.iter().enumerate() {
            if mask & (1 << bit) != 0 {
                cell.set(cell.get() + seconds);
            }
        }
    }

    /// The cells as the VM's profile reads them (lua_profilesplit's `hostkinds`)
    pub fn as_ptr(&self) -> *const f64 {
        self.cells.as_ptr() as *const f64
    }

    /// Each kind's total so far
    pub fn totals(&self) -> [f64; 5] {
        std::array::from_fn(|bit| self.cells[bit].get())
    }
}

/// What a cold or first-use charge of the engine is for ([`Touched::cold`]): an instance's first touch in a frame (by slot; its class
/// names it in the totals), a member's first use, an engine path's first use, a class's first clone or destroy in the frame. A profile
/// keeps who paid each and whether other code touched the same thing in the frame (profiler::engine_cold): a cut of the payer leaves
/// a shared charge to the other.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ColdKey {
    Instance(u32, ClassId),
    Member(MemberUse, crate::reflection::NameId),
    Path(Path),
    Cloned(ClassId),
    Destroyed(ClassId),
}

/// The frame each instance slot was last touched in (one per engine: the scheduler owns it, the Instance API's C metamethods
/// share it like the work cell), the frame each member was last used in ([`Touched::first_use`]), each engine path and each class
/// was last cloned or destroyed in ([`Touched::first_path`]), and the frame each instance entered the Workspace in.
pub struct Touched {
    frame: u32,
    stamps: Vec<u32>,
    /// By [`MemberUse`], the frame each member name was last used in; the last slot of each is [`CHILD_LOOKUP`]'s
    members: [Vec<u32>; 3],
    /// What a member's first use in a frame costs, by [`MemberUse`]
    first_prices: [f64; 3],
    /// Whether the most recent [`Touched::touch`] found its instance cold
    last_cold: bool,
    /// The first-touch price of each class ([`cold_touch`]) by ClassId: [`COLD_TOUCH`] for every class until [`Touched::set_prices`]
    prices: Vec<f64>,
    /// What each class's cold write extras are priced as ([`ColdKind`]), by ClassId
    kinds: Vec<ColdKind>,
    /// The serial of the last instance a touch found cold ([`COLD_TOUCH_ALONE`]), and the frame it had last been touched in before
    last_serial: u32,
    last_prev: u32,
    /// The frame each [`Path`] was last used in
    paths: [u32; PATHS],
    /// By ClassId, the frame a class was last cloned and destroyed in, and what its first clone and destroy of a frame, a cold
    /// template's clone and a live instance's destroy add ([`first_cloned`], [`first_destroyed`], [`clone_cold`], [`destroy_live`])
    cloned: Vec<u32>,
    destroyed: Vec<u32>,
    class_prices: Vec<[f64; 4]>,
    /// By slot, the frame its instance entered the Workspace in (0: not in it)
    entered: Vec<u32>,
    /// The Workspace, once the world is made (entering it is [`Path::Enter`])
    pub workspace: Option<crate::InstanceId>,
    /// By ClassId, whether it hangs on a part a move carries ([`Touched::moved_cold`]) and whether it is a particle emitter
    hangs: Vec<bool>,
    emitters: Vec<bool>,
    /// By ClassId, what a move inside the DataModel costs an instance of it ([`reparented`]), and the WorldRoot class (a part's world)
    reparented: Vec<f64>,
    pub world_root: Option<ClassId>,
    /// PivotTo's name, priced by the method call itself ([`Touched::pivot_cold`])
    pub pivot_to: Option<crate::reflection::NameId>,
    /// The instance the PivotTo being priced moves (lua/instance.rs pivot_cold), until the engine's Luau writes it: that write
    /// changes a part as a script's would ([`part_changes`]), where each part a PivotTo carries counts one change
    pub pivot_root: Option<crate::InstanceId>,
    /// What the run's scripts have asked the engine to do so far ([`EngineOps`], [`Touched::count`]), and the scheduler's mute:
    /// above 0 while the engine's own Luau prices a whole call (Scheduler::muted), whose inner operations are not counted
    ops: EngineOps,
    muted: std::rc::Rc<std::cell::Cell<u32>>,
    /// The engine's charges by kind (Scheduler::kinds): the cold ones are booked there as they are priced
    work_kinds: Option<std::rc::Rc<WorkKinds>>,
    /// A profile keeps who paid each cold charge and what else touched its object (profiler::engine_cold)
    pub profiling: bool,
}

impl Touched {
    /// `muted`: the scheduler's mute (Scheduler::muted). Kind totals are attached only while a work profile runs.
    pub fn new(muted: std::rc::Rc<std::cell::Cell<u32>>) -> Self {
        Touched {
            frame: 1,
            stamps: Vec::new(),
            members: [Vec::new(), Vec::new(), Vec::new()],
            first_prices: [FIRST_READ, FIRST_WRITE, FIRST_CALL],
            last_cold: false,
            prices: Vec::new(),
            kinds: Vec::new(),
            last_serial: 0,
            last_prev: 0,
            paths: [0; PATHS],
            cloned: Vec::new(),
            destroyed: Vec::new(),
            class_prices: Vec::new(),
            entered: Vec::new(),
            workspace: None,
            hangs: Vec::new(),
            emitters: Vec::new(),
            reparented: Vec::new(),
            world_root: None,
            pivot_to: None,
            pivot_root: None,
            ops: EngineOps::default(),
            muted,
            work_kinds: None,
            profiling: false,
        }
    }

    /// Starts (`Some`) or ends (`None`) the booking of cold charges by kind and by payer: a work profile does it while it steps
    pub(crate) fn profile_work(&mut self, kinds: Option<std::rc::Rc<WorkKinds>>) {
        self.profiling = kinds.is_some();
        self.work_kinds = kinds;
    }

    /// Books a cold or first-use charge of `seconds` that the caller adds to its price: the cold kind of the engine's charges, and while a
    /// profile steps, who paid it for `key` (profiler::engine_cold). Nothing while the engine's own Luau prices a whole call (its charges
    /// are dropped). Returns `seconds`.
    /// (these hooks run in every engine call: inline they are a flag test, and what a profile keeps is out of line, as their bodies
    /// inline cost an unprofiled engine call 5 % in cycle measurements)
    #[inline]
    pub fn cold(&mut self, seconds: f64, key: ColdKey) -> f64 {
        // (kinds and attributions are a profile's: nothing to keep when none steps)
        if self.profiling {
            self.cold_profiled(seconds, key);
        }
        seconds
    }

    #[cold]
    #[inline(never)]
    fn cold_profiled(&self, seconds: f64, key: ColdKey) {
        let Some(totals) = &self.work_kinds else { return };
        if seconds > 0.0 && self.muted.get() == 0 {
            totals.add(kind::COLD, seconds);
            crate::profiler::attribution::engine_cold(key, seconds);
        }
    }

    /// Books the kinds of a charge of `seconds` the caller makes (a Studio-era or a borrowed price: [`kind`] bits); nothing while the
    /// engine's own Luau prices a whole call
    #[inline]
    pub fn book_kinds(&self, kinds: u8, seconds: f64) {
        if self.profiling {
            self.book_kinds_profiled(kinds, seconds);
        }
    }

    #[cold]
    #[inline(never)]
    fn book_kinds_profiled(&self, kinds: u8, seconds: f64) {
        let Some(totals) = &self.work_kinds else { return };
        if kinds != 0 && seconds != 0.0 && self.muted.get() == 0 {
            totals.add(kinds, seconds);
        }
    }

    /// [`book_kinds`](Self::book_kinds) of kinds worked out only while a profile keeps them (a lookup by class or name)
    #[inline]
    pub fn book_kinds_with(&self, seconds: f64, kinds: impl FnOnce() -> u8) {
        if self.profiling {
            self.book_kinds_profiled(kinds(), seconds);
        }
    }

    /// Code touched `key`'s object again in this frame (a profile tells a charge other code shared: profiler::engine_touch)
    #[inline]
    fn seen(&self, key: ColdKey) {
        if self.profiling {
            Self::seen_profiled(key);
        }
    }

    #[cold]
    #[inline(never)]
    fn seen_profiled(key: ColdKey) {
        crate::profiler::attribution::engine_touch(key);
    }

    /// Counts an engine operation a script asked for ([`EngineOps`]). Not while the engine's own Luau prices a whole call: a
    /// PivotTo's GetDescendants and its parts' CFrame writes are the pivot's, as the work clock drops their charges.
    #[inline]
    pub fn count(&mut self, add: impl FnOnce(&mut EngineOps)) {
        if self.muted.get() == 0 {
            add(&mut self.ops);
        }
    }

    /// What the run's scripts have asked the engine to do so far
    pub fn ops(&self) -> EngineOps {
        self.ops
    }

    /// Prices each class's first touch ([`cold_touch`]), cold writes ([`ColdKind`]) and its share of a frame's first clones and destroys.
    pub fn set_prices(&mut self, reflection: &Reflection) {
        let classes = 0..reflection.class_count() as ClassId;
        self.kinds = classes.clone().map(|class| ColdKind::of(reflection, class)).collect();
        self.prices = classes.clone().map(|class| cold_touch(reflection, class)).collect();
        self.class_prices = classes
            .clone()
            .map(|class| [first_cloned(reflection, class), first_destroyed(reflection, class), clone_cold(reflection, class), destroy_live(reflection, class)])
            .collect();
        const HANGS: [&str; 8] = ["Attachment", "ParticleEmitter", "Beam", "Trail", "Light", "Decal", "Texture", "DataModelMesh"];
        self.hangs = classes.clone().map(|class| HANGS.iter().any(|kind| reflection.is_a(class, kind))).collect();
        self.emitters = classes.clone().map(|class| reflection.is_a(class, "ParticleEmitter")).collect();
        self.reparented = classes.map(|class| reparented(reflection, class)).collect();
        self.world_root = reflection.class_id("WorldRoot");
        self.pivot_to = reflection.name_id("PivotTo");
    }

    /// What the frame's first use of `path` adds ([`PATH_FIRST`]); later uses in the frame nothing.
    #[inline]
    pub fn first_path(&mut self, path: Path) -> f64 {
        let stamp = &mut self.paths[path as usize];
        if *stamp == self.frame {
            self.seen(ColdKey::Path(path));
            0.0
        } else {
            *stamp = self.frame;
            self.cold(PATH_FIRST[path as usize], ColdKey::Path(path))
        }
    }

    fn class_price(&self, class: ClassId, which: usize) -> f64 {
        self.class_prices.get(class as usize).map_or(0.0, |prices| prices[which])
    }

    /// What a cold instance of `class` adds when a move carries it: `part` for a part, `attached` for what hangs on one (an
    /// attachment, an emitter, a beam, a trail, a light, a decal, a mesh), nothing for a model or folder.
    pub fn moved_cold(&self, class: ClassId, part: f64, attached: f64) -> f64 {
        match self.kind(class) {
            ColdKind::Part => part,
            _ if self.hangs.get(class as usize).copied().unwrap_or(false) => attached,
            _ => 0.0,
        }
    }

    /// Whether `class` is an emitter BulkMoveTo's cold parts carry ([`BULK_COLD_EMITTER`])
    pub fn is_emitter(&self, class: ClassId) -> bool {
        self.emitters.get(class as usize).copied().unwrap_or(false)
    }

    /// What one instance of a template being cloned adds: its class's first clone in the frame ([`first_cloned`]) and, when no code
    /// touched it in this frame, a cold template's read ([`clone_cold`]); marks it touched. `cold` says it for the root, which the
    /// call itself touched.
    pub fn cloned_source(&mut self, index: u32, class: ClassId, cold: Option<bool>) -> f64 {
        let mut price = 0.0;
        let slot = class as usize;
        if self.cloned.len() <= slot {
            self.cloned.resize(slot + 1, 0);
        }
        if self.cloned[slot] != self.frame {
            self.cloned[slot] = self.frame;
            price += self.cold(self.class_price(class, 0), ColdKey::Cloned(class));
        } else {
            self.seen(ColdKey::Cloned(class));
        }
        let marked = self.mark(index);
        if cold.unwrap_or(marked) {
            price += self.cold(self.class_price(class, 2), ColdKey::Instance(index, class));
        } else {
            self.seen(ColdKey::Instance(index, class));
        }
        price
    }

    /// What destroying one instance adds: its class's first destroy in the frame ([`first_destroyed`]) and, when it has been in the
    /// Workspace since an earlier frame, the engine's work it undoes ([`destroy_live`]). Clears its Workspace stamp.
    pub fn destroyed(&mut self, index: u32, class: ClassId) -> f64 {
        let mut price = 0.0;
        let slot = class as usize;
        if self.destroyed.len() <= slot {
            self.destroyed.resize(slot + 1, 0);
        }
        if self.destroyed[slot] != self.frame {
            self.destroyed[slot] = self.frame;
            price += self.cold(self.class_price(class, 1), ColdKey::Destroyed(class));
        } else {
            self.seen(ColdKey::Destroyed(class));
        }
        let live = self.entered.get(index as usize).is_some_and(|&entered| entered != 0 && entered != self.frame);
        if live {
            price += self.class_price(class, 3);
        }
        if let Some(entered) = self.entered.get_mut(index as usize) {
            *entered = 0;
        }
        price
    }

    /// The instance in slot `index` entered the Workspace (`inside`) or left it.
    pub fn set_entered(&mut self, index: u32, inside: bool) {
        let slot = index as usize;
        if self.entered.len() <= slot {
            if !inside {
                return;
            }
            self.entered.resize(slot + 1 + slot / 2, 0);
        }
        self.entered[slot] = if inside { self.frame } else { 0 };
    }

    /// Whether the instance the last cold touch found had been touched in the frame before (moved every frame: [`Touched::mark_moved`])
    pub fn last_cold_was_recent(&self) -> bool {
        self.last_prev + 1 >= self.frame
    }

    /// Marks the instance in slot `index` touched in this frame without a price; whether no code touched it in this frame or the one
    /// before (a part a script moves every frame keeps the engine's state for it warm: [`PIVOT_COLD_PART`], [`BULK_COLD_PART`]).
    #[inline]
    pub fn mark_moved(&mut self, index: u32) -> bool {
        let slot = index as usize;
        if self.stamps.len() <= slot {
            self.stamps.resize(slot + 1 + slot / 2, 0);
        }
        let last = std::mem::replace(&mut self.stamps[slot], self.frame);
        last + 1 < self.frame
    }

    /// Marks the instance in slot `index` touched in this frame without a price; whether it was cold.
    #[inline]
    pub fn mark(&mut self, index: u32) -> bool {
        let slot = index as usize;
        if self.stamps.len() <= slot {
            self.stamps.resize(slot + 1 + slot / 2, 0);
        }
        if self.stamps[slot] == self.frame {
            false
        } else {
            self.stamps[slot] = self.frame;
            true
        }
    }

    /// What `class`'s cold write extras are priced as ([`cold_write_extra`])
    #[inline]
    pub fn kind(&self, class: ClassId) -> ColdKind {
        self.kinds.get(class as usize).copied().unwrap_or(ColdKind::Other)
    }

    /// What a move inside the DataModel costs an instance of `class` ([`reparented`])
    pub fn reparented(&self, class: ClassId) -> f64 {
        self.reparented.get(class as usize).copied().unwrap_or(REPARENTED_INSTANCE)
    }

    /// Whether the most recent touch found its instance cold (a SetAttribute's namecall touches before its body runs); asking clears it,
    /// so a later call that did no touch of its own (an engine-internal SetAttribute) never inherits it.
    pub fn take_last_cold(&mut self) -> bool {
        std::mem::take(&mut self.last_cold)
    }

    /// Frame `frame` begins (every instance is cold again); a profile settles the frame's cold charges.
    pub fn begin_frame(&mut self, frame: u64) {
        self.frame = frame as u32 + 1;
        if self.profiling {
            crate::profiler::attribution::engine_frame();
        }
    }

    /// What a script's use of member `name` (a read, a write or a method call) adds for the engine's code being cold: its first use in
    /// the frame costs [`FIRST_READ`], [`FIRST_WRITE`] or [`FIRST_CALL`], later ones nothing. A read that finds a child passes
    /// [`CHILD_LOOKUP`].
    #[inline]
    pub fn first_use(&mut self, kind: MemberUse, name: crate::reflection::NameId) -> f64 {
        // (every engine call takes this: a profile's bookkeeping is a separate instantiation, out of line)
        if self.profiling { self.first_use_profiled(kind, name) } else { self.first_use_in::<false>(kind, name) }
    }

    #[cold]
    #[inline(never)]
    fn first_use_profiled(&mut self, kind: MemberUse, name: crate::reflection::NameId) -> f64 {
        self.first_use_in::<true>(kind, name)
    }

    #[inline]
    fn first_use_in<const PROFILE: bool>(&mut self, kind: MemberUse, name: crate::reflection::NameId) -> f64 {
        let price = self.first_prices[kind as usize];
        if price == 0.0 {
            return 0.0;
        }
        let frame = self.frame;
        let stamps = &mut self.members[kind as usize];
        let slot = if name == CHILD_LOOKUP { 0 } else { name as usize + 1 };
        if stamps.len() <= slot {
            stamps.resize(slot + 1 + slot / 2, 0);
        }
        if stamps[slot] == frame {
            if PROFILE {
                Self::seen_profiled(ColdKey::Member(kind, name));
            }
            0.0
        } else {
            stamps[slot] = frame;
            if PROFILE {
                self.cold_profiled(price, ColdKey::Member(kind, name));
            }
            price
        }
    }

    /// The first-touch price of `class` ([`cold_touch`]) if the instance in slot `index` has not been touched in this frame, else 0;
    /// marks it touched. A part or other non-GUI instance not made next to the last instance a touch found cold (`serial`, its
    /// creation order) pays [`COLD_TOUCH_ALONE`].
    #[inline]
    pub fn touch(&mut self, index: u32, class: ClassId, serial: u32) -> f64 {
        // (every engine call takes this: a profile's bookkeeping is a separate instantiation, out of line)
        if self.profiling { self.touch_profiled(index, class, serial) } else { self.touch_in::<false>(index, class, serial) }
    }

    #[cold]
    #[inline(never)]
    fn touch_profiled(&mut self, index: u32, class: ClassId, serial: u32) -> f64 {
        self.touch_in::<true>(index, class, serial)
    }

    #[inline]
    fn touch_in<const PROFILE: bool>(&mut self, index: u32, class: ClassId, serial: u32) -> f64 {
        let slot = index as usize;
        if self.stamps.len() <= slot {
            self.stamps.resize(slot + 1 + slot / 2, 0);
        }
        if self.stamps[slot] == self.frame {
            self.last_cold = false;
            if PROFILE {
                Self::seen_profiled(ColdKey::Instance(index, class));
            }
            0.0
        } else {
            self.last_prev = std::mem::replace(&mut self.stamps[slot], self.frame);
            self.last_cold = true;
            // (made next to the last cold one, or touched in the frame before: the plain first touch)
            let together = serial.abs_diff(self.last_serial) == 1 || self.last_prev + 1 >= self.frame;
            self.last_serial = serial;
            let price = self.prices.get(class as usize).copied().unwrap_or(COLD_TOUCH);
            let price = if together || price != COLD_TOUCH { price } else { COLD_TOUCH_ALONE };
            if PROFILE {
                self.cold_profiled(price, ColdKey::Instance(index, class));
            }
            price
        }
    }
}

// Second pass (2026-10-23): the instance operations are priced from an op_cost probe, one body run in
// Studio and in ree, the median of seven rounds less an empty loop, in an empty test place (edit) and in
// a lobby playtest's client (a game's real runtime, the figures used
// where they differ; a property write in a running place costs about 60 % of an edit write). Every figure
// below that says "client" is that bench's number.
//
// Third pass (2026-11-07): "the parity bench" is tools/cost-parity, whose workloads run unchanged in ree and in a rendered client of a
// game's match place in roblox-sim (Studio 0.744.0.7441077, Deferred signals, two sessions, the median of 9 rounds), each figure less
// the Luau around the call at these prices (fit.py residuals). A third of its rows, chosen by a hash of their names, is held out: no
// price is set from them, and a figure quoted from one says so. Studio playtest clients, where most prices were measured before, read
// many engine calls 1.3-4x dearer: their Explorer, Properties window and script debugger listen to the DataModel.

/// A property or member read: a number, bool or Vector3 property of a part, a frame or an effect, 25.5-30.4 ns (median 29.4) beyond the
/// Luau that reads it, in the parity bench. What the value is adds to it: [`read_value`].
pub const READ: f64 = 0.0315e-6;

/// What a read of `value` adds to [`READ`]: the engine makes a new value for most types on each read (the same bench: a CFrame
/// 45.8-53.6 ns in all, a Color3 40.5-42.5, a NumberSequence or ColorSequence 61.4-66.1, a UDim2 41.0, a Vector2 40.3, a string 37.5-40.3,
/// an Instance 35.1-46.9, an EnumItem 40.0; a Vector3 is the VM's own value and costs nothing more).
pub fn read_value(value: Option<&crate::Value>) -> f64 {
    use crate::Value;
    match value {
        None | Some(Value::Nil | Value::Bool(_) | Value::Number(_) | Value::Vector3(_) | Value::Lua(_)) => 0.0,
        Some(Value::String(_)) => READ_STRING_EXTRA,
        Some(Value::EnumItem(_)) => 11.0e-9,
        Some(Value::Instance(_) | Value::NoInstance) => READ_INSTANCE_EXTRA,
        Some(Value::Vector2(_)) => 11.0e-9,
        Some(Value::Color3(_)) => 19.0e-9,
        Some(Value::UDim(_) | Value::UDim2(_) | Value::Rect(_) | Value::NumberRange(_) | Value::Font(_) | Value::Content(_)) => 12.0e-9,
        Some(Value::NumberSequence(_) | Value::ColorSequence(_)) => 40.0e-9,
        // (a CFrame, BrickColor, PhysicalProperties and the other rbx-dom datatypes)
        Some(Value::Dom(_)) => 26.0e-9,
    }
}

/// What a computed property getter (`derived.rs`, `world/mod.rs`) adds beyond [`READ`]: Euler decomposition for
/// `Orientation` / `Rotation` and parent CFrame composition for Attachment `World*` reads.
pub fn read_property_extra(property: &str) -> f64 {
    match property {
        "Orientation" | "Rotation" | "WorldOrientation" | "WorldRotation" => 17.0e-9,
        "WorldPosition" | "WorldAxis" | "WorldSecondaryAxis" => 10.0e-9,
        "WorldCFrame" => 36.0e-9,
        _ => 0.0,
    }
}

/// What a string read adds (Name, ClassName, Text, Image: 37.5-40.3 ns in all) and an Instance read (Parent, PrimaryPart,
/// LocalPlayer: 35.1-46.9): [`read_value`]'s, and the Name, ClassName and Parent reads the Instance API serves itself
pub const READ_STRING_EXTRA: f64 = 12.0e-9;
pub const READ_INSTANCE_EXTRA: f64 = 13.0e-9;
/// A child found by name as a member (`part.Attachment`: 40.0 ns in all)
pub const READ_CHILD_EXTRA: f64 = 11.0e-9;
/// What reaching a member by its name with the first letter lower-cased (`part.anchored`, `part:getFullName()`) adds to
/// the member's own price: the engine looks the key up, finds no member and no child, and looks again (a headless client,
/// roblox-sim, Studio 0.744, the minimum of 7 rounds of 200,000, less the exact name's: reads of a property, an event and a
/// method and a method call +33 to +47 ns on a part without children; a read pays 2.4 ns more for each child the failed
/// child lookup passes, not charged, as no child lookup is charged by its children)
pub const LOWER_FIRST: f64 = 40.0e-9;
/// A property write nothing in the engine reacts to, and a write of the value it holds (client: CFrame written
/// to what it holds 52 ns, Bone.Transform and Motor6D.Transform 55-68 ns)
pub const WRITE_QUIET: f64 = 0.055e-6;
/// A method call the table below does not name, beyond the Luau around it (the parity bench, its whole-corpus sessions: FindFirstChild's
/// first of 20 children 58 ns, FindFirstAncestorOfClass 60; a Studio playtest client read 45 over the Luau of the older prices)
pub const CALL: f64 = 0.058e-6;
/// HttpService:JSONEncode and JSONDecode by what they handle, beyond [`method`]'s price for the call ([`JSON_CALL`]), in seconds: a byte
/// of the JSON text written or parsed and a value written or made (each table, string, number and boolean of the document). What a
/// decode makes and an encode's text are the engine's own allocations (a host function's: lua_sethostcfunction), so these prices carry
/// them, and the VM's library counts leave them out. The parity bench's `json` rows, lists of 10 to 10,000 records (about 105 bytes and
/// 9 values a record), fitted the prices (tools/cost-parity/fit.py sizes): a flat 0.69 us read 413 bytes and 468 KB alike.
pub fn json(bytes: usize, values: u64, decode: bool) -> f64 {
    let (byte, value) = if decode { (JSON_DECODE_BYTE, JSON_DECODE_VALUE) } else { (JSON_ENCODE_BYTE, JSON_ENCODE_VALUE) };
    1e-9 * (bytes as f64 * byte + values as f64 * value)
}
pub const JSON_CALL: f64 = 0.35e-6;
/// Nanoseconds a byte and a value cost an encode and a decode
pub const JSON_ENCODE_BYTE: f64 = 0.0;
pub const JSON_ENCODE_VALUE: f64 = 55.5;
pub const JSON_DECODE_BYTE: f64 = 8.15;
pub const JSON_DECODE_VALUE: f64 = 1.50;

/// An instance destroyed, beyond leaving the DataModel ([`moved`]): a Folder out of the tree 0.41 us with the call, in the parity bench.
/// The Studio capture's 1.0 (a grass patch 4.9 us an instance with its leaving) had the Explorer's work in it.
pub const DESTROYED: f64 = 0.32e-6;
/// What a GUI object in the DataModel adds when destroyed (the same bench: a Frame under a ScreenGui 2.51 us, of which leaving 1.17), and a
/// part (a Part in the workspace 2.14, leaving 1.47)
pub const DESTROYED_GUI: f64 = 0.89e-6;
pub const DESTROYED_PART: f64 = 0.22e-6;
/// One element of a list a method returns (the parity bench: GetChildren of 20 0.34 us,
/// GetDescendants of 110 1.94 us; EditableMesh GetFaces and GetColors 0.01-0.02 us an id in a Studio capture)
pub const LISTED: f64 = 0.0135e-6;
/// ReflectionService: a class's entry GetClass or GetClasses makes ([`REFLECTED_CLASS`]), what its Display table adds, and a class
/// GetClasses tests against its filter. A playing client (roblox-sim, Studio 0.744, a headless client of an empty place; os.clock
/// around single calls, the median of 15; the parity bench's ReflectionService rows are newer than its reference sessions):
/// GetClasses() of its 656 classes 1.235 ms, with ExcludeDisplay 0.866 ms, with IsA = "BasePart" (19 entries) 39.5 us;
/// GetClass("Part") 2.22 us, of which 0.28 is the lookup ([`method`]).
pub const REFLECTED_CLASS: f64 = 1.55e-6;
pub const REFLECTED_CLASS_DISPLAY: f64 = 0.56e-6;
pub const REFLECTED_CLASS_TESTED: f64 = 0.006e-6;
/// A LogService logging call (Output, Info, Warn, Error, Log), whole: the same client, `LogService:Info("L {a}", { a = 1 })` 1.10 us
/// with the Luau around it (20 calls a sample, the median of 15 samples), the table constructor about 0.1 of that.
pub const LOG_CALL: f64 = 1.0e-6;
/// A PhysicsService group change that works out every part's CollisionGroupId in the Workspace again (Register, Create, Unregister,
/// Remove, Rename: services/physics.rs), beyond the call: a part it looks at, and one whose id it changes. A server (roblox-sim,
/// Studio 0.747, a Script of a headless play session; os.clock around single calls, the median of 15, two passes of each scene):
/// with 0, 1,000, 5,000 and 20,000 parts whose id stays, a register 0.5, 2.2, 8.7-8.8 and 34.7-34.9 us (unregister and rename
/// alike), so 1.7 ns a part, in models or not; with parts of the group, whose id changes each time, 360-364 us at 1,000 and
/// 1,866-1,910 at 5,000 (0.36-0.38 us a part more; 0.43-0.44 at 20,000, 0.45 in models). CollisionGroupSetCollidable also costs a
/// server 1.7 ns a part (43 us at 20,000), which ree, keeping no ids by mask, does not charge.
/// The Workspace's other instances cost a change next to nothing (a register and an unregister 4.0-6.8 us beside 100,000
/// Folders, 1.8-3.4 beside none), and ree looks at none of them.
pub const COLLISION_GROUP_PART: f64 = 1.7e-9;
pub const COLLISION_GROUP_CHANGED: f64 = 0.38e-6;
/// A cast (Raycast, Spherecast, Blockcast, Shapecast) beyond [`method`]'s price for the call, by the parts its path meets up to its hit,
/// whether its filter admits them or not, and none behind the hit (world/query.rs charge_cast: a client's cast stops there, measured
/// on Studio 0.747 for all four): one that meets any pays [`RAY_MET`], and each part after the first [`RAY_CROSSED`] (but a shape
/// cast's parts tied with its hit: [`SPHERE_CROSSED`], [`BLOCK_CROSSED`]). The parity
/// bench's rays, on geometry of their own that both sides find the same parts in (Meta.queries), beyond the Luau around the call: a
/// miss with an Include filter 0.27 us, an Exclude filter of the one part a ray crosses 0.45, a ray through a stack of 24 parts 1.50 and
/// the same with an Include filter of one of them 1.35; held out: a miss with no filter 0.17, a hit on one part 0.41. (The bench's sim
/// side ran each group of families as a job of one session, and until each job took its scene out again the method rows met every
/// earlier job's parts too: three stacks of 24, which the first fit read as a ray's or a query's dearness.)
pub const RAY_MET: f64 = 0.175e-6;
pub const RAY_CROSSED: f64 = 0.0424e-6;
/// What a spatial query that takes parts into the spatial index pays for them, beyond its own price (world/query.rs take_in_of:
/// the parts that came into the world of the WorldRoot it is called on since a query of that world or the physics step last took
/// that world's in, camera_moves.rs `Cameras::unindexed` for the Workspace's and `Cameras::unindexed_models` for a WorldModel's;
/// a Raycast or any other query of a WorldModel takes in its world's), in a
/// playing client (Studio 0.747, a headless client; os.clock around K parts parented into the Workspace, a Raycast or
/// GetPartBoundsInBox and the parts parented out again, less the same without the query and the query alone, the median of 31 or
/// 41 samples, two passes beside no other parts and beside 100,000 parts away): K = 1 1.48-1.61 us, 10 9.2-10.6, 100 95-122,
/// 1,000 1,110-1,380, beside 100,000 parts as beside none, so [`QUERY_TAKEN_PART`] a part and [`QUERY_TAKE_IN`] once. Moving
/// parts costs the next query nothing (1, 10 or 100 moved: within 0.3 us of a query after none, with no growth), and so does
/// taking them out (100 or 1,000 parented out: within 0.06 us a part of none, either way). GetLargestCutoffDistance and the
/// PlayerMouse's rays take nothing in and pay nothing for it. ArePartsTouchingOthers pays the same for the parts it takes in, a
/// WorldModel's own world's on a WorldModel (world/region.rs; Studio 0.748, a playing client, the same method, 31 samples: K = 10,
/// 100 and 1,000 12.7, 120 and 1,233 us in the Workspace, 12.2, 110 and 1,080 in a WorldModel under ReplicatedStorage).
pub const QUERY_TAKE_IN: f64 = 0.5e-6;
pub const QUERY_TAKEN_PART: f64 = 1.1e-6;
/// A step up the tree of the walks that put what a spatial query keeps in the Workspace's order (world/broadphase.rs
/// sort_by_order: up from the parts at once until their paths meet; a collision-group change's parts whose ids change too,
/// services/physics.rs), past [`QUERY_ORDER_FREE`] steps for each part it orders: ree's
/// own host work, as a playing client's queries cost the same however deep their parts are (roblox-sim, Studio 0.747, a headless
/// client; os.clock around 100 calls, the median of 15 less an empty loop: GetPartBoundsInBox finding 100 parts 12.1 to 12.2 us
/// in a folder of the Workspace, 12.2 at the bottom of a 1,000-deep folder chain, 12.5 at 3,000, 17.3 with each part 300 deep in
/// a chain of its own; 1,000 parts 17.7 to 25.0 in a folder, 22.9 1,000 deep, 18.0 each 30 deep; a Raycast through the first
/// hundred 1.7 to 2.7 us in every scene). Parts in folders and models take about a step each, within the allowance, so their
/// queries pay nothing more; parts whose paths meet only far up (each in a deep chain of its own, or one in the Workspace and
/// one deep down) pay for the walk ree still takes, measured in `ree run` with os.realclock: 10 to 35 ns of host time a step
/// (one part 10,000 deep beside one in the Workspace 10, 1,000 parts each 100 deep 15 to 21, 100 parts each 1,000 deep 24 to
/// 35, a Raycast entering those 15 to 26), 20 on average.
pub const QUERY_ORDER_STEP: f64 = 20e-9;
pub const QUERY_ORDER_FREE: usize = 2;
/// Camera:GetPartsObscuringTarget beyond its call (world/camera.rs parts_obscuring), in a playing client (roblox-sim, Studio 0.747, a
/// headless client; os.clock around calls, 15 samples less an empty loop, two rounds of 90 cases: 0 to 100 walls across a point's
/// segment, in a folder or at the bottom of chains of 10 to 300 folders, ignore lists of 1 to 1,000 parts elsewhere or of the walls'
/// folder, 1 to 1,000 points behind the same walls, behind a wall each or behind none). A client casts again for each part it lists,
/// checking the parts before each hit against its growing ignore list, so a call grows with the cube of the parts a segment crosses
/// and with their depth: 10 walls 12 us, 20 32 to 34, 50 214 to 392, 100 1.0 to 1.9 ms; 20 walls 300 folders deep 2.0 ms; 10 walls
/// 100 deep behind an ignore list of 1,000 9.2 to 9.4 ms; a point behind nothing 0.17 us. The prices: [`OBSCURING_CALL`] (0.495 us
/// with ree's call to the method, 0.1), [`OBSCURING_CAST`] (from the points behind nothing), and [`OBSCURING_LISTED`],
/// [`OBSCURING_CHECKED`] and [`OBSCURING_STEP`] (times [`filter_step`] for the part's depth) fitted for the least relative error:
/// within x0.40 to x1.80 of the 180 cases, median 0.93 (the client's two rounds differ by up to 1.9x). The prices before (0.8 us a
/// point, a ray's for the parts it crossed, 1.1 a part listed), fitted on up to 10 parts in the Workspace, read 0.002 to 5.0 of
/// these. An entry of the points that is no Vector3 costs [`OBSCURING_SKIPPED`] (a number 14 ns, a string 17, an Instance or a
/// CFrame 25, a Vector2 36: 1,000 of them beside a point), a point that is not finite [`OBSCURING_NOT_FINITE`] (ten 1.6 us, one 0.7).
pub const OBSCURING_CALL: f64 = 0.4e-6;
pub const OBSCURING_CAST: f64 = 0.175e-6;
pub const OBSCURING_LISTED: f64 = 0.418e-6;
pub const OBSCURING_CHECKED: f64 = 0.070e-6;
pub const OBSCURING_STEP: f64 = 2.7;
pub const OBSCURING_SKIPPED: f64 = 0.02e-6;
pub const OBSCURING_NOT_FINITE: f64 = 0.1e-6;
/// An entry of Camera:GetPartsObscuringTarget's or GetLargestCutoffDistance's ignore list read (world/mod.rs ignore_list), in a
/// playing client (roblox-sim, Studio 0.747, a headless client; os.clock around 10 to 200 calls in pcall, the median of 15 less
/// a pcall of an empty function, lists of 0 to 10,000 parts, two sessions): an Instance 19 ns on a GetPartsObscuringTarget that
/// goes on and 25 on a GetLargestCutoffDistance (10 when the points after the list raise), a nil hole 46 and 48. When an entry
/// is no Instance the call raises, and each entry before it cost 177 to 221 ns in all three of these lists' calls (BulkMoveTo's
/// part list 200 in that session, 339 in the next) and the bad entry itself the error's own few microseconds; a client reads
/// the rest of the list too (40 to 84 ns an entry after a bad first one), which ree does not read and does not charge.
pub const IGNORE_LISTED: f64 = 0.022e-6;
pub const IGNORE_HOLE: f64 = 0.047e-6;
pub const LIST_CAST_FAILED: f64 = 0.19e-6;
/// Camera:GetLargestCutoffDistance beyond its call (world/camera_moves.rs), in a playing client (roblox-sim, Studio 0.747, a headless
/// client; os.clock around 200 calls on a new camera, the median of 21 samples less an empty loop, two passes of each scene): its
/// five rays missing 1.12 us whether the Workspace holds 0, 1,000, 5,000 or 20,000 parts off them (in models or not: the
/// broadphase), of which ree's call to the method counts 0.07; the five each meeting a wall 1.80, so 0.137 a ray that meets a
/// part ([`CUTOFF_MET`]; a Raycast's hit over its miss read 0.19 in the same session). Each further part a ray meets before its
/// hit, unmeasured, pays a Raycast's [`RAY_CROSSED`].
pub const CUTOFF_CALL: f64 = 1.055e-6;
pub const CUTOFF_MET: f64 = 0.137e-6;
/// A PlayerMouse ray read (Target, target, TargetSurface; Hit and hit add [`MOUSE_FRAME`]: world/mouse.rs) beyond [`READ`], in a
/// playing client (roblox-sim, Studio 0.747, a headless client; os.clock around 2,000 reads, the median of 21 samples less an empty
/// loop and a plain read, two passes; a scratch probe through tools/parity's capture harness). Each read casts the mouse's ray its
/// 10,000 studs: a miss 0.44 us for Target, 0.48 for TargetSurface and 0.64 for Hit; the parts off the ray cost nothing (5,000
/// behind the camera, 200 in TargetFilter). The parts it meets up to its hit (the hit, and those under TargetFilter before it,
/// which a client casts past again) are paid as a ray's: [`RAY_MET`] for the first and [`MOUSE_FILTERED`] for each after it; a hit
/// adds [`MOUSE_HIT`]. Measured: a hit 0.24 to 0.31 us (more with parts close behind it; the parts further behind cost nothing, a
/// stack of 50 what one of 10 does), parts under TargetFilter on a miss one 0.21, five 0.76, 24 2.69, and 12 before a hit 1.58
/// more than the hit alone. Fitted within x0.83 to x1.08 per read, x1.06 on average, over 18 scenes of the three members.
pub const MOUSE_RAY: f64 = 0.46e-6;
pub const MOUSE_FRAME: f64 = 0.19e-6;
pub const MOUSE_HIT: f64 = 0.10e-6;
pub const MOUSE_FILTERED: f64 = 0.115e-6;
/// Reading a PlayerMouse's Icon, UnitRay and Origin beyond a plain read of the mouse (X; world/mouse.rs), in a playing client
/// (roblox-sim, Studio 0.747, a headless client; os.clock around 2,000 reads, the median of 21 samples less an empty loop, three
/// sessions): Icon 4.48 to 4.65 us in all with no Icon set, 4.66 to 5.20 with one, UnitRay 0.113 to 0.123 and Origin 0.143 to 0.152,
/// where X reads 0.056 to 0.058 (and a part's Transparency 0.026).
pub const MOUSE_ICON: f64 = 4.49e-6;
pub const MOUSE_UNIT_RAY: f64 = 0.06e-6;
pub const MOUSE_ORIGIN: f64 = 0.09e-6;
/// A Spherecast's and a Blockcast's part tied with its hit, after the first, that its filter admits (the bench's
/// `workspace.Spherecast.hit_stack` and `Blockcast.hit_stack`, a 1-stud ball and a 2-stud box into the stack of 24 coincident parts:
/// 9.68 and 6.35 us at the fastest of six sessions, their misses 0.18 and 0.22 like a ray's). A shape tests more of each part it
/// admits than a ray does; a part its filter leaves out costs it a ray's [`RAY_CROSSED`] (a headless 0.747 client, the stack with an
/// Include filter of one part, or 23 excluded walls before the hit: 0.02 to 0.03 us a part for a Raycast, Spherecast, Blockcast and
/// Shapecast alike), and one behind the hit nothing. Shapecast's parts tied with its hit, unmeasured, borrow the Blockcast's (a
/// defaulted price).
pub const SPHERE_CROSSED: f64 = 0.405e-6;
pub const BLOCK_CROSSED: f64 = 0.259e-6;
/// A part GetPartBoundsInBox returns and one GetPartBoundsInRadius returns (the same geometry: an empty box 0.15 us whether it is 4
/// studs or 40 across, a line of 10 parts in a box 1.54, a radius that touches 2 parts 0.33; held out: a box of 24 parts 3.80)
pub const QUERY_BOX_PART: f64 = 0.14e-6;
pub const QUERY_RADIUS_PART: f64 = 0.09e-6;
/// What an overlap query whose geometry is not finite costs beyond its call (world/query.rs), in a playing client (roblox-sim,
/// Studio 0.747, a headless client; os.clock around calls in pcall, 15 samples less an empty loop, beside 0, 1,000, 5,000 and
/// 10,000 parts, which change nothing): GetPartBoundsInBox with a frame that is not finite 2.9 to 3.1 us where one that finds
/// a part reads 0.40, GetPartBoundsInRadius with a position that is not finite 2.7 to 3.0 ([`QUERY_NOT_FINITE`]); a NaN radius
/// 1.3 to 1.5 and a box with a NaN or infinite side 1.5 to 2.3 ([`QUERY_SIDE_NOT_FINITE`]); neither tests a part.
pub const QUERY_NOT_FINITE: f64 = 2.7e-6;
pub const QUERY_SIDE_NOT_FINITE: f64 = 1.3e-6;
/// An error an engine function raises, beyond its call: an Instance's method, a property read or write, a service's method, a
/// library's function, a datatype's, a spatial query's (lua/cfunction.rs charge_raised, where ree raises one). The error is the
/// cost, whatever raised it: in a playing client and a server alike (roblox-sim, Studio 0.747, headless; os.clock around 10 to
/// 20,000 calls in pcall, 15 samples less an empty pcall, three sessions), past the same call going on, a plain error (the
/// engine's own words, no position) 3.1 to 3.7 us ([`RAISED`]): FindFirstChild(nil) 3.4-3.6, GetAttribute(nil) 3.5, IsA(nil)
/// 3.4, WaitForChild(nil) 3.4, a method called with `.` and no instance 3.2-3.3, an unknown method 3.2, an unknown member read
/// 3.3-3.4 or written 3.2 (50 folders deep 3.8-3.9, its longer name), GetService of no service 3.5-3.6, GetPropertyChangedSignal
/// of no property 3.1, JSONDecode of bad JSON 2.7, BulkMoveTo of mismatched lists 4.0, require(5) 3.4-5.3, an enum's unknown item 3.2,
/// the spatial queries 3.1 to 4.4 (Raycast 3.4, a box 3.1-3.4, Spherecast with a NaN radius 4.0-4.4, GetPartsObscuringTarget
/// 3.7-4.3); one at the calling line (Luau's words for an argument check, positioned) 2.0 to 3.0 ([`RAISED_AT_CALLER`]):
/// task.cancel, task.delay, task.wait and task.spawn 2.4-2.7, typeof() 2.2, a Parent written a number 2.8-2.9, Instance.new(nil)
/// 2.7-2.8, SharedTable.new and size 2.3-2.4, Random.new and NextNumber 2.4-2.5, DateTime 2.5, utf8.nfcnormalize 2.7,
/// collectgarbage 2.4, Signal.Wait 2.9, iterating an Instance 2.1, CFrame.new 2.3-2.4. ree charged the call
/// alone (0.004 to 0.45 us) but for the spatial queries (3.4 since 2026-11-23, now this). Not charged here: a refused property
/// write costs a client more, 4.5 to 4.8 us (read only, an enum's unknown item, a locked or circular Parent) and 7.4 to 7.7 (a
/// value of the wrong type for a datatype or string property), where this charges RAISED; a script's own error (error(),
/// assert, arithmetic, a builtin's argument) 1.8 to 2.4, and an error raised by a datatype's method or operator or a proxy
/// value's member 2.2 to 2.9, which ree raises in the VM, mlua or the engine's Luau and does not charge.
pub const RAISED: f64 = 3.4e-6;
pub const RAISED_AT_CALLER: f64 = 2.5e-6;
/// A RaycastParams' or OverlapParams' instance list a query reads (world/query.rs list_reads: FilterDescendantsInstances,
/// ExcludeInstances, IncludeInstances), in a playing client (roblox-sim, Studio 0.747, a headless client; os.clock around calls,
/// 15 samples less an empty loop, two rounds; lists of 0, 10, 100, 1,000 and 10,000 parts away from the query, which it finds
/// nothing near): a GetPartBoundsInBox 0.13 to 0.2 us with an empty list, 0.31 with 10, 1.03 with 100, 7.96 with 1,000 and 78
/// with 10,000, whichever list, Exclude or Include; a Raycast's miss 0.30, 0.51, 1.30, 8.73 and 83.5: 7.8 to 8.3 ns an entry at
/// every query ([`PARAMS_LISTED`]) and about 0.1 us a list that holds any ([`PARAMS_LIST`]). Assigning or reading a list costs
/// a client its copy ([`PARAMS_WRITTEN`] and below).
pub const PARAMS_LIST: f64 = 0.1e-6;
pub const PARAMS_LISTED: f64 = 7.8e-9;
/// The copy a RaycastParams' or OverlapParams' instance list costs a script when it is assigned or read (the prelude's
/// FilterParamsType ListCopy) or added to (AddToFilter), past the proxy's host call (LUAU_HOSTCALL, what a write of IgnoreWater
/// costs a client too). In a playing client and a server (roblox-sim, Studio 0.747, headless; os.clock around 13 to 20,000 calls
/// in pcall, 15 samples less an empty pcall, two sessions; lists of 0, 1, 10, 100, 1,000 and 10,000 parts): assigning
/// FilterDescendantsInstances 0.14 us with an empty list, 0.22 with 10, 1.37-1.41 with 100, 13.1-13.4 with 1,000 and 137-139
/// with 10,000, either params type ([`PARAMS_LIST_WRITE`], [`PARAMS_WRITTEN`] an entry: x0.71 for the empty list, x0.90 to x1.13
/// the others); ExcludeInstances and IncludeInstances 0.09, 0.22, 1.57, 16.7-17.8 and 206-210 ([`PARAMS_FILTER_WRITTEN`] an
/// entry: a client's grows from 13 ns an entry at 10 to 20.6 at 10,000, and this is its 1,000's; x0.82 to x1.23); reading any of
/// them 0.03, 0.12-0.14, 0.9-1.1, 9.6-10.1 and 114-119 ([`PARAMS_LIST_READ`], [`PARAMS_READ`] an entry; x0.76 to x1.20);
/// AddToFilter of a list onto an empty one 0.22, 0.39, 1.58, 13.4 and 141, its write included (the write's prices for the
/// entries it adds, x0.91 to x1.08: a client appends them, and adding one costs it 0.03 to 0.6 us onto a list of 0 or 10,000
/// alike). Main charged the host call alone (0.018 us) whatever the length, where its copy of 10,000 entries took 27 us of host
/// time. A table given to RaycastParams.new costs nothing: a client ignores it (a 0.747 client and server read the new object's
/// defaults).
pub const PARAMS_LIST_WRITE: f64 = 0.08e-6;
pub const PARAMS_WRITTEN: f64 = 13.6e-9;
pub const PARAMS_FILTER_WRITTEN: f64 = 17e-9;
pub const PARAMS_LIST_READ: f64 = 0.02e-6;
pub const PARAMS_READ: f64 = 11e-9;
/// A step of a client's FilterDescendantsInstances check of a query's part (world/query.rs Filter::check): for each instance of
/// the list in its order until one is the part or an ancestor of it, a walk up from the part, to the top of the tree for each
/// instance before it. In a playing client (roblox-sim, Studio 0.747, a headless client; os.clock around GetPartBoundsInBox
/// finding 1 or 10 parts at the bottom of a chain of 1 to 998 folders, lists of 1 to 1,000 parts elsewhere, an Exclude filter;
/// less the same query with no list and the list's read, [`PARAMS_LISTED`]), each step of each walk: 0.17 to 0.29 ns with 3 to
/// 30 ancestors, 0.40 at 60, 0.50 at 100 and 0.80 to 0.89 from 200 to 1,000 (a deep part's ancestors no longer in a cache):
/// [`QUERY_FILTER_STEP`] to 30 ancestors, [`QUERY_FILTER_DEEP_STEP`] from 200, and between them in proportion ([`filter_step`]).
/// 100 parts 1,000 deep behind a list of 10,000 cost one query 826 ms on a client.
pub const QUERY_FILTER_STEP: f64 = 0.25e-9;
pub const QUERY_FILTER_DEEP_STEP: f64 = 0.85e-9;
pub fn filter_step(ancestors: u32) -> f64 {
    let deep = ((f64::from(ancestors) - 30.0) / 170.0).clamp(0.0, 1.0);
    QUERY_FILTER_STEP + deep * (QUERY_FILTER_DEEP_STEP - QUERY_FILTER_STEP)
}
/// An instance an ExcludeInstances or IncludeInstances check looks at, walking up from a query's part until either list names
/// one (world/query.rs Filter::check), for each list that holds any: [`QUERY_LISTED_STEP`] and [`QUERY_LISTED_SEARCH`] for each
/// doubling of the list (the same sessions, 10 parts 98 to 998 folders deep: 0.83 to 1.05 ns a step with a list of 1, 1.8 to 2.1
/// with 10, 2.8 to 3.0 with 100, 4.0 to 4.3 with 1,000 and 6.4 to 7.8 with 10,000; [`listed_step`]).
pub const QUERY_LISTED_STEP: f64 = 0.85e-9;
pub const QUERY_LISTED_SEARCH: f64 = 0.3e-9;
pub fn listed_step(entries: usize) -> f64 {
    QUERY_LISTED_STEP + QUERY_LISTED_SEARCH * (entries.max(1) as f64).log2()
}
/// A part an overlap query's filter leaves out, past its walk (world/query.rs overlapping; the same sessions): 10 parts left out
/// by an Include filter of nothing 0.60 us, 100 parts by an Exclude of their folder 5.5 to 5.9; a part found costs
/// [`QUERY_BOX_PART`] instead.
pub const QUERY_FILTERED_PART: f64 = 0.058e-6;
/// A step of the PlayerMouse's TargetFilter check (world/mouse.rs pointed): up from each part its ray meets before its hit to
/// the TargetFilter, and from the hit to the top of the tree. The same client, Target with 20 walls under the TargetFilter on its
/// ray and a wall behind them: 2.5 us with the walls 2 folders deep, 23.4 at 300 and 52 at 1,000 (2.5 to 3.5 ns a step).
pub const MOUSE_FILTER_STEP: f64 = 2.8e-9;
/// A step of GetLargestCutoffDistance's ignore list check (world/camera_moves.rs blocks): up from a part one of its rays meets
/// before its hit to an ignored instance or the top of the tree. The same client, ten walls across the five rays under an ignored
/// folder: 3.1 us 2 deep, 23.6 at 300 and 59.3 at 1,000 (1.1 to 1.4 ns a step of each part each ray meets); with an empty list
/// the call costs the same at any depth (2.1 to 2.3 us), so no walk is taken.
pub const CUTOFF_IGNORE_STEP: f64 = 1.2e-9;
/// A part GetPartsInPart returns (the 23 others of a stack 17.3 us), and its own work by the query part's size: [`method`]'s price for
/// a part of no size, and [`QUERY_PART_SIZE`] for each stud of the square root of its box's surface area (beyond the Luau around the
/// call, touching nothing: a 2-stud cube 0.27 us, 12 studs 0.45, 20 studs 0.95, 40 studs 1.26, a 2 x 2 x 40 rod 0.46; held out: a 6-stud
/// cube 0.25, a 4-stud ball 0.25, a 40 x 2 x 40 slab 0.48, a 2-stud cube with an Include filter 0.45)
pub const QUERY_PART_EXACT: f64 = 0.59e-6;
pub const QUERY_PART_SIZE: f64 = 0.011e-6;
/// The Region3 queries (world/region.rs) beyond their call, in a playing client (Studio 0.748, os.clock around 200 calls, the median
/// of 15 samples less an empty loop, three passes within 5% of each other, GetPartBoundsInBox timed beside them): each part a query
/// finds in its region ([`REGION_FOUND`]), each it lists ([`REGION_LISTED`] more), each its ignored instance, ignore list or
/// whitelist leaves out ([`REGION_EXCLUDED`] more; an ignored instance the region holds none of costs nothing); IsRegion3Empty*
/// lists none, and pays [`REGION_ANY`] once when it finds a part. Its list is read at [`IGNORE_LISTED`] an entry (world/mod.rs
/// ignore_list; 1,000 entries over an empty region 24 to 28.5 us on this client, a list of 50 0.02 us an entry), and its checks
/// of the parts it finds against its ignored instance, ignore list or whitelist walk up each part for each instance of the list
/// in turn, at [`filter_step`] a step (world/region.rs Keep::keeps). FindPartsInRegion3 over a
/// line of 0, 1, 10, 50 and 200 parts 0.40, 0.68, 1.84, 7.2 and 27.9 us (GetPartBoundsInBox 0.24, 0.45, 1.6, 6.7, 26.8); 50 parts
/// with maxParts 5 1.97, with 20 3.77, 200 with 5 5.2; 10 and 50 parts under the ignored folder 1.08 and 3.4, beside an unrelated one
/// 1.9 and 7.3; an ignore list of 50 unrelated entries 3.0 over 10 parts, a whitelist of none 0.98; IsRegion3Empty over 0, 1, 10, 50
/// and 200 parts 0.42, 0.59, 0.72, 1.44 and 4.57, over 50 ignored 3.37; IsRegion3EmptyWithIgnoreList of 50 entries over none 1.06.
pub const REGION_FOUND: f64 = 0.020e-6;
pub const REGION_LISTED: f64 = 0.118e-6;
pub const REGION_EXCLUDED: f64 = 0.040e-6;
pub const REGION_ANY: f64 = 0.15e-6;
/// Each part a Region3 query meets in its region and leaves out for having CanQuery and CanCollide both off (world/region.rs;
/// the same client and method): 10, 100 and 1,000 such parts in the region cost FindPartsInRegion3 1.23, 7.37 and 56.6 us where
/// none cost 0.40, IsRegion3Empty the same (least squares 0.056 us a part); 196 of them 0.1 past its face 11.2 (0.055 a part).
/// A part waiting for the spatial index costs nothing (1,000 of them 0.40): the client's broadphase does not hold it, nor do
/// ree's trees until a query of its world takes it in (world/broadphase.rs); nor do a WorldModel's parts in a Workspace query's
/// region (196 of them 0.42), which ree's Workspace index keeps in a tree of their own.
pub const REGION_HIDDEN: f64 = 0.056e-6;
/// Each part a Region3 query's broadphase gives it whose box lies outside its region but within 0.305 of it along each axis
/// (world/region.rs BAND: [`REGION_NEAR`]), and each one further out ([`REGION_FAR`]), in a playing client (Studio 0.748,
/// os.clock around 100 calls, the median of 9 samples less an empty loop, three passes, FindPartsInRegion3 with maxParts 1 and
/// IsRegion3Empty alike, regions at 4 mod 8): 196 half-stud cubes spread over an 8-stud region's face, 0.01 to 0.30 past it,
/// 16.8 to 18.6 us where none cost 0.40, 0.1 before its min face 17.6, 133 cubes of 0.05 0.25 or 0.3 past both faces of an edge
/// 11.8, with CanQuery or CanCollide off alike (0.084 to 0.091 us a part: more than a part found, [`REGION_FOUND`], the 196
/// inside it 4.55); 0.31 to 3 past it, in a cell of the region's, 2.4 (0.0102 a part); 1 past it in no cell of the region's
/// nothing. Parts stacked in one spot cost more (196 0.01 to 0.3 past 22.4, inside 9.6, 1 to 3 past 6.4), which is not modeled.
pub const REGION_NEAR: f64 = 0.086e-6;
pub const REGION_FAR: f64 = 0.0102e-6;
/// ArePartsTouchingOthers beyond its call (world/region.rs; the same client and method): a list with any entries ([`TOUCHING_LIST`]),
/// each entry ([`IGNORE_LISTED`] as it is read, world/region.rs touching_entries, and [`TOUCHING_ENTRY`] more once the entries
/// passed their check), each listed part it checks (until one touches) and each nearby part it tests against one. An empty list
/// 0.27 us, one part with nothing near 0.57, one touching another 0.82, 10 and 30 with nothing near 1.82 and 4.67, 10 and 30 whose
/// first touches 1.36 and 2.53 (0.06 us an entry in all); a Folder after 1,000 parts raises for 30 to 37 us (the raise
/// included: the read alone, 0.022 an entry). A call that raises costs [`TOUCHING_RAISED`] beyond its call and [`RAISED`] (which
/// lua/cfunction.rs charge_raised charges for every engine error), more than a Region3 query: an overlapIgnored that is no number
/// 7.0 to 8.4 us, a list whose first entry is no Instance 7.5 to 9.2, one whose first entry is a Folder 4.6 to 6.0, past an empty
/// pcall (median 7.2), less the call's price (6.8 in all).
pub const TOUCHING_LIST: f64 = 0.15e-6;
pub const TOUCHING_ENTRY: f64 = 0.038e-6;
pub const TOUCHING_CHECKED: f64 = 0.09e-6;
pub const TOUCHING_RAISED: f64 = 3.4e-6;
/// The nearby parts ArePartsTouchingOthers meets for a listed part (world/region.rs; the same client and method): each it tests
/// ([`TOUCHING_PAIR`], plus [`TOUCHING_TESTED`] once for a listed part it tests against any), each it skips for being listed
/// ([`TOUCHING_LISTED_NEAR`]) or for having CanCollide off ([`TOUCHING_OFF_NEAR`]). One part beside 1, 10, 30, 100 and 300 others
/// 0.1 away 0.83, 2.43, 5.70, 17.4 and 52.5 us (nothing near 0.58); among 10, 30, 100 and 300 parts with CanCollide off 1.35,
/// 2.11, 7.71 and 17.3 (least squares 0.057 us a part); 10, 30, 100 and 300 parts in one spot, all listed, 12.5, 65.2, 1,142 and
/// 6,898 (each meets the others: least squares 0.077 us a part met, 0.069 to 0.117 by size).
pub const TOUCHING_PAIR: f64 = 0.17e-6;
pub const TOUCHING_TESTED: f64 = 0.08e-6;
pub const TOUCHING_LISTED_NEAR: f64 = 0.077e-6;
pub const TOUCHING_OFF_NEAR: f64 = 0.057e-6;
/// Each part ArePartsTouchingOthers' walk is given for a listed part that lies further than 0.3125 from its box (world/region.rs
/// REACH; the same client and method as [`REGION_NEAR`]): an 8-stud cube beside 196 half-stud cubes spread over its face 0.315
/// to 4 away 2.65 to 3.0 us where nothing near cost 0.66 (0.0107 a part), 0.2 to 0.31 away (tested) 29.5 to 31.7; two-stud cubes
/// stacked 0.5 to 2 away, 100 3.8 and 300 5.4, 4 away (in no cell of the listed part's) nothing.
pub const TOUCHING_FAR: f64 = 0.0107e-6;
/// Each part put in a WorldModel's own index when the index is made, at the WorldModel's first Region3 query or walked
/// ArePartsTouchingOthers in the DataModel, or made again after its change log overflowed (world/broadphase.rs `ModelIndex`, with
/// [`FIND_NODE`] for each instance the walk of its world looks at; world/region.rs), but a part a query of that WorldModel took in
/// and paid for since the index last took its log. This is ree's own cost, not a client's: a client puts a world's parts in
/// its index as they come in, at the price of taking them in ([`QUERY_TAKEN_PART`]), or in the physics step. Measured on
/// ree's host (a release build, the fastest of six runs): a WorldModel of 200,000 parts made and queried once took 123 ms more
/// than made alone, one of 200,000 Folders under 1 ms more.
pub const MODEL_INDEX_PART: f64 = 0.6e-6;
/// FindFirstChild and the other finds: a child looked at past the first (the parity bench: the 20th of 20 children 73 ns, the first 44,
/// so 1.55 ns a child), and a node a recursive find looks at (a miss over 1,110 nodes 4.2 us: 3.8 ns). A collision-group change's walk
/// of a WorldModel's descendants for its parts pays [`FIND_NODE`] for each other instance it looks at (services/physics.rs, the price
/// borrowed).
pub const FIND_CHILD: f64 = 1.55e-9;
pub const FIND_NODE: f64 = 3.8e-9;
/// GetTagged: an instance it lists (the parity bench: 10 tagged parts 0.92 us beyond the Luau around the call; the first fit read 1.89,
/// in a client holding three earlier jobs' tagged parts too)
pub const TAGGED_LISTED: f64 = 0.086e-6;

/// QueryDescendants: a node it looks at and a node it returns (client: ">>Part" over 100 descendants 3.6 us,
/// ">>Model > Part#Part3" 1.0 us for 10 matches, edit 2.3 and 1.2)
pub const QUERIED: f64 = 0.008e-6;
pub const QUERY_FOUND: f64 = 0.015e-6;
/// An attribute set to another value, beyond the call: SetAttribute of a string 0.43 us, a new key and its removal 0.43 a call, in
/// the parity bench (a Studio playtest read 0.77-0.83: its Properties window listens); to the value it holds 0.077
/// in all ([`ATTRIBUTE_SAME`] beyond the call)
pub const ATTRIBUTE_CHANGED: f64 = 0.38e-6;
pub const ATTRIBUTE_SAME: f64 = 0.032e-6;
/// One attribute GetAttributes lists (the same bench: five 0.205 us beyond the luau)
pub const ATTRIBUTE_LISTED: f64 = 0.032e-6;
/// Signal:Connect and Connection:Disconnect (the parity bench: Connect and Disconnect of a
/// BindableEvent, a part's Changed and a Once 0.50-0.51 us together; split 60/40 as a Studio playtest's 0.78 was),
/// GetPropertyChangedSignal beyond the call (0.42 us in all)
pub const CONNECT: f64 = 0.34e-6;
pub const DISCONNECT: f64 = 0.22e-6;
pub const PROPERTY_SIGNAL: f64 = 0.34e-6;
/// BindableEvent:Fire with nothing connected, beyond the call (the same bench: 52 ns in all; a Studio playtest client read 161)
pub const FIRE: f64 = 0.007e-6;
/// A property write that fires Changed or a GetPropertyChangedSignal signal someone is connected to, per signal fired (the
/// op_cost probe's signal rows, a match place's client, two sessions, over the same write with nothing connected: Changed +227..354
/// ns (the lobby client 318, edit 474), the property signal +79..213, both together +365..410). The handler runs later, deferred,
/// and is charged to its own thread; this is the writer's cost of queueing it.
pub const SIGNAL_FIRED: f64 = 0.20e-6;

/// The instance events whose firing and handlers Studio's client was measured on (a handler_cost probe, a match place's
/// client, three runs, 100 instances triggered a frame by a thread
/// resumed in PreSimulation, one handler each). A handler's own thread (its resume, the first line it runs, its return) is a cost
/// the firer's os.clock bracket never sees: the last handler's first line came 1.46-2.3 us a fire after the trigger returned.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SignalKind {
    /// Changed
    Changed,
    /// GetPropertyChangedSignal
    Property,
    /// AttributeChanged and GetAttributeChangedSignal
    Attribute,
    ChildAdded,
    /// one per instance of the subtree and listening ancestor
    DescendantAdded,
    /// (measured with the second handler_cost probe, [`EXTRA_HANDLER_QUEUED`])
    ChildRemoved,
    DescendantRemoving,
    AncestryChanged,
    /// BindableEvent:Fire with a connection
    Bindable,
    /// A loopback run's remote event reaching its handlers (OnServerEvent, OnClientEvent): priced as an unmeasured event until
    /// tools/cost-parity measures a remote's dispatch
    Remote,
    /// other events: fired as before, unpriced
    Unmeasured,
}

/// What a second handler and each one after it on the same signal adds (a second handler_cost probe, a match place's client, three
/// runs, a Changed signal with 10 handlers on each of 100 parts and with 100 handlers on each of 10 parts): the trigger's queueing grew by
/// 0.45 us a further handler (10 handlers 4.55-4.99 us a fire, 100 handlers 43.0-45.1; one handler 0.51-0.80, so the fire's own part is
/// the 0.20 [`SIGNAL_FIRED`]) and each handler's thread cost 0.87-1.04 us (median 0.95) where a lone handler's cost 1.6-2.8, so the first
/// handler pays the dispatch of the fire and the others only their thread. A lone handler is still priced as the earlier probes had
/// it: their hot loop of two writes read the queueing 0.23-0.35 us, this one and the first handler_cost probe's 100 instances a frame 0.5-0.8.
/// (Studio's figures scaled by [`signal_costs`]'s quarter: no row of the parity bench has several handlers)
pub const EXTRA_HANDLER_QUEUED: f64 = 0.45e-6 * 0.247;
pub const EXTRA_HANDLER_THREAD: f64 = 0.95e-6 * 0.247;
/// One more handler of a frame event (PreSimulation, Heartbeat, PreRender, ...), charged for each handler after the first (the first
/// one's dispatch is the frame's) and booked to it: 100 handlers on one event, the span from the first handler's first line to the last,
/// over 99 (a match place's client, three runs: PreSimulation 0.73, Heartbeat 0.63, PreRender 0.71 us).
/// The parity bench (tools/cost-parity, a rendered client of a game's match place in roblox-sim): 100 Heartbeat handlers 0.49-0.51 us apart.
pub const FRAME_HANDLER: f64 = 0.50e-6;

/// What a fire of `kind` with a handler connected costs the firer, and what each handler's thread costs when it runs (seconds). Queueing,
/// per fire (the trigger less the trigger with nothing connected, medians of three runs): Changed and the property signal keep
/// [`SIGNAL_FIRED`]'s 0.20 us (its loop of two writes read +0.23..0.35 and +0.08..0.21; this probe's 100 a frame 0.53 and 0.60, noisier), AttributeChanged
/// 0.63 / 0.51 / 0.66 and GetAttributeChangedSignal 0.82 / 0.55 / 0.55 (0.55 us), ChildAdded 0.40 / 0.38 / 0.66 (0.40), DescendantAdded
/// 0.27 / 0.25 / 0.41 (0.27). Handler thread, per fire (last handler's first line less the trigger's end, less 0.02 us for the handler's
/// own instructions): Changed 1.97 / 1.89 / 2.24 (1.83 as the median of the property signal's 1.87 too), AttributeChanged 1.71 / 1.62 /
/// 2.06 and GetAttributeChangedSignal 1.86 / 1.72 / 1.93 (1.78), ChildAdded 1.53 / 1.46 / 1.89 (1.50), DescendantAdded 1.46 / 1.45 /
/// 1.69 (1.44).
///
/// Those were Studio playtest clients, whose handler threads carry Studio's script hooks. The parity bench (a rendered client of a game's
/// match place in roblox-sim, Deferred signals) reads a heard BindableEvent:Fire through its handler at 0.64 us, of which the call is
/// 0.05: 0.57 for the queueing and the thread, a quarter of Studio's 2.31. Its other kinds' rows are in the held-out third, so every
/// kind's pair is Studio's scaled by that quarter (`STUDIO_HOOKS`), but Changed's queueing, which the bench reads directly (a heard
/// Transparency write 0.20 us over the write: [`SIGNAL_FIRED`]).
pub fn signal_costs(kind: SignalKind) -> (f64, f64) {
    const STUDIO_HOOKS: f64 = 0.247;
    let (queue, thread) = match kind {
        SignalKind::Changed | SignalKind::Property => return (SIGNAL_FIRED, 1.83e-6 * STUDIO_HOOKS),
        SignalKind::Attribute => (0.55e-6, 1.78e-6),
        SignalKind::ChildAdded => (0.40e-6, 1.50e-6),
        SignalKind::DescendantAdded => (0.27e-6, 1.44e-6),
        // (three runs of the second handler_cost probe in a match place's client: queueing 0.42 / 0.35 / 0.65 / 0.58 us, handler thread
        // 1.47 / 1.58 / 2.17 / 1.73)
        SignalKind::ChildRemoved => (0.42e-6, 1.50e-6),
        SignalKind::DescendantRemoving => (0.35e-6, 1.58e-6),
        SignalKind::AncestryChanged => (0.65e-6, 2.17e-6),
        SignalKind::Bindable => (0.58e-6, 1.73e-6),
        // (an event no bench measured: a heard BindableEvent's, a borrowed price: signal_kinds)
        SignalKind::Unmeasured | SignalKind::Remote => (0.58e-6, 1.73e-6),
    };
    (queue * STUDIO_HOOKS, thread * STUDIO_HOOKS)
}

/// Where [`signal_costs`]' two prices came from (cost.rs [`kind`] bits for the queueing and for each handler's thread): Changed's
/// queueing the parity bench measured; a heard BindableEvent's pair is the bench's total split as Studio split it; the other measured
/// kinds are Studio's figures scaled by the bench's Bindable row (Studio-era); an event no bench measured borrows the Bindable's
/// (defaulted, and counted: a profile's `unmeasured signal fires`)
pub fn signal_kinds(signal: SignalKind) -> SignalKinds {
    match signal {
        SignalKind::Changed | SignalKind::Property => SignalKinds { queue: 0, thread: kind::STUDIO },
        SignalKind::Bindable => SignalKinds { queue: 0, thread: 0 },
        SignalKind::Unmeasured | SignalKind::Remote => SignalKinds { queue: kind::DEFAULTED, thread: kind::DEFAULTED },
        _ => SignalKinds { queue: kind::STUDIO, thread: kind::STUDIO },
    }
}

/// The [`kind`] bits of [`signal_costs`]' two prices (none: 0, what a run keeps when no profile reads them)
#[derive(Clone, Copy, Default)]
pub struct SignalKinds {
    /// The firer's queueing of the event
    pub queue: u8,
    /// Each handler's thread
    pub thread: u8,
}

/// What resuming a thread parked in task.wait (or wait) and in task.delay costs a client beyond the thread's own instructions, booked
/// to the line it waited on (scheduler.rs run_timers): the parity bench's `task.wait.resume_spacing` and `task.delay.resume_spacing`,
/// 100 threads resumed in one frame, the span from the first one's stamp to the last's over 99, less the instructions each runs (a
/// rendered client of a game's match place, roblox-sim, Studio 0.744, the fastest of four sessions). Charged nothing before: 30,000
/// resumptions were free.
pub const RESUMED_WAIT: f64 = 0.35e-6;
pub const RESUMED_DELAY: f64 = 0.20e-6;
/// A thread parked in a signal's Wait, resumed by a fire (the bench's `task.signal_wait.resume_spacing`, the same way: 100 threads
/// waiting on one BindableEvent, 515 ns apart at the fastest session, less their instructions), booked to its Wait; a handler's thread
/// keeps [`signal_costs`]' price
pub const RESUMED_SIGNAL_WAIT: f64 = 0.445e-6;
/// A thread made for task.spawn / defer / delay (the parity bench: task.spawn(noop) 0.72 us,
/// task.delay 0.86 (the timer: [`DELAYED`] more); a Studio playtest client read 1.15), and a thread handed to them: a client resumes a
/// coroutine parked in a yield through task.spawn at what a new function's thread costs (notes/macro pathprobe, rendered match-place
/// clients: 1.10 us hot against a function's 0.95)
pub const THREAD: f64 = 0.72e-6;
/// The engine's call of a render-step binding (BindToRenderStep): the time between one binding's last line and the next's first, in
/// rendered match-place clients (notes/macro pathprobe, two sessions: 1.8 us, of which ree counts 0.26 in the resumption)
pub const RENDER_STEP_DISPATCH: f64 = 1.55e-6;
pub const DELAYED: f64 = 0.14e-6;
/// BulkMoveTo, a part in the call (client, 100 parts: FireCFrameChanged 0.42 us a part, FireAllEvents 1.03; a
/// loop of CFrame writes 1.10)
/// (the parity bench: BulkMoveTo of 10 parts with FireCFrameChanged 1.05 us a part; the events'
/// share kept as the playtest measured it)
pub const BULK_MOVED: f64 = 0.88e-6;
pub const BULK_MOVED_ALL_EVENTS: f64 = 2.58e-6;
/// BulkMoveTo's part list read (world/mod.rs bulk_moves; the sessions of [`IGNORE_LISTED`]): each entry 43 ns on a call that then
/// raises because the lists' lengths differ and 66 ns on one that raises at a part that is no BasePart (each entry of the
/// list, wherever the bad part stands), and a nil hole 81 ns on a call that moves (its parts pay [`BULK_MOVED`])
pub const BULK_LISTED: f64 = 0.055e-6;
pub const BULK_HOLE: f64 = 0.081e-6;
/// Model:PivotTo, a part it moves (client: 0.48 us in the workspace; 0.15 a call), a Part:PivotTo 1.14 us
/// (the parity bench, a rendered client of a game's match place: Model:PivotTo of 10 parts 10.65 us, Part:PivotTo 1.14)
pub const PIVOT_PART: f64 = 1.05e-6;
pub const PIVOT_BASE: f64 = 0.15e-6;
/// ... out of the workspace, a part (a CFrame write there 0.16 us, and the walk)
pub const PIVOT_LOOSE: f64 = 0.20e-6;
pub const PIVOT_SINGLE: f64 = 1.14e-6;
/// ... each descendant of the model that is no part, which its walk looks at (a playing client, roblox-sim, Studio 0.747, headless;
/// os.clock around single Model:PivotTo calls of a Model holding one anchored part with N = 1,000 and 10,000 Folders, Attachments
/// or Welds under it, the median of 15 samples, less the same call with none: 11.0 to 11.9 ns each whatever their class; 2026-11-24)
pub const PIVOT_DESCENDANT: f64 = 11.3e-9;
/// Model:ScaleTo, an instance it looks at, and what a real rescale adds for the ones it rewrites (client, 100
/// Parts: to the scale they have 0.19 us each; a rescale in the workspace 1.16, out of the DataModel 0.28; a
/// fallback rig's 225 ScaleTo(1) out of it: a rig of 1 MeshPart, 7 Bones and 19
/// Attachments 14.7 us, a rig of 7 Parts, 6 Motor6Ds and 19 Attachments 26.9 us, which an Attachment or
/// Bone at 0.54 and a Motor6D at 2.0 us fit)
pub const SCALE_SCANNED: f64 = 0.19e-6;
pub const SCALE_PART_PLACED: f64 = 0.97e-6;
pub const SCALE_PART_LOOSE: f64 = 0.09e-6;
pub const SCALE_ATTACHMENT: f64 = 0.35e-6;
pub const SCALE_JOINT: f64 = 1.8e-6;
/// Model:GetPivot / GetScale / GetBoundingBox (CFrame return: 35 ns beyond the call) and GetExtentsSize (native Vector3 return: 8 ns)
pub const MODEL_QUERY: f64 = 0.035e-6;
pub const MODEL_EXTENTS_QUERY: f64 = 0.008e-6;
/// A Parent change that neither enters nor leaves the DataModel, beyond its two ancestor chains ([`reparent_chains`]) and the instances
/// it carries, the written one included ([`reparented`], [`WORLD_WORKSPACE_PART`], [`PLACED_PART`]): the parity bench's Part into
/// another Folder of the workspace and back, 0.51 us a move, less what its chains (4 ancestors each, 3 of them shared) and the Part
/// cost at those prices. One between two places outside the DataModel: the bench's 0.15 us (chains of one ancestor each, none
/// shared), less the same, at [`REPARENTED_LOOSE_INSTANCE`] an instance.
pub const REPARENTED: f64 = 0.51e-6 - reparent_chains(4, 4, 3) - REPARENTED_INSTANCE - REPARENTED_KEPT;
pub const REPARENTED_LOOSE: f64 = 0.15e-6 - reparent_chains(1, 1, 0) - REPARENTED_LOOSE_INSTANCE;
/// What every Parent change spends on the ancestor chains of the old parent and the new one (`old` and `new` ancestors each, the
/// DataModel or a parentless root included, `kept` of them in both): a playing client (roblox-sim, Studio 0.747, a headless client;
/// os.clock around single moves of a Part, the median of 15; docs/engine.md "Moves and ancestor reads") moved it between two
/// Folders at the bottom of one chain of D Folders in ReplicatedStorage or the Workspace (D = 1 to 5,000: 0.6 us at 1, 3.0 at 100,
/// 30.6 at 1,000, 170 at 5,000), between the bottoms of two such chains (0.45 us at 1, 20.7 at 100, 2,853 at 1,000, 92,900 at
/// 5,000), and between a branch of Do Folders and a Folder beside it, below a chain of Ds (Ds and Do 1 to 2,000, both ways, and
/// between two branches). Its walks grow with each shared ancestor, each it leaves and each it joins, and with each ancestor it
/// leaves times the new chain's length and each it joins times the old chain's ([`REPARENT_LEFT_PAIR`], [`REPARENT_JOINED_PAIR`]:
/// a move between two chains 1,000 deep costs a client 2.9 ms). Fitted for the least relative error over the 35 cases: within
/// x0.62 to x1.54 of each, x0.97 at the median.
pub const fn reparent_chains(old: usize, new: usize, kept: usize) -> f64 {
    let (left, joined) = ((old - kept) as f64, (new - kept) as f64);
    kept as f64 * REPARENT_KEPT_LEVEL
        + left * REPARENT_LEFT_LEVEL
        + joined * REPARENT_JOINED_LEVEL
        + left * new as f64 * REPARENT_LEFT_PAIR
        + joined * old as f64 * REPARENT_JOINED_PAIR
}
pub const REPARENT_KEPT_LEVEL: f64 = 26.5e-9;
pub const REPARENT_LEFT_LEVEL: f64 = 5.2e-9;
pub const REPARENT_JOINED_LEVEL: f64 = 23.0e-9;
pub const REPARENT_LEFT_PAIR: f64 = 0.95e-9;
pub const REPARENT_JOINED_PAIR: f64 = 1.85e-9;
/// A place a client's search for a child passes to take it out of its parent's list (a Parent write, a Remove or a Destroy), in the
/// order the client keeps the list in: it looks at the last child first, then from the start (dom.rs ClientOrder, Dom::reparent).
/// A playing client (roblox-sim, Studio 0.747, a headless client; os.clock around single removals of Folders from one parentless
/// Folder of 1,000, 100,000 or 1,000,000, the median of 9 to 15) took one out at the start or the end in 0.2-0.4 us however long the
/// list (Destroy 0.6-1.0), and one 50,000 places in 9.4 (1,000,000 lists: 499,999 places 96, 999,000 places 187), as much after a
/// removal moved the last child 50,000 places in (9.6-10.1 each of 40 last children after it), and 100,000 taken out first to last
/// in 323 ms (a Destroy of their Folder the same): 0.20 ns a place, within x0.69 to x1.41 of each of 44 such rows, x1.07 at the median.
pub const CHILD_SEARCHED: f64 = 0.2e-9;
/// What each instance a Parent change carries costs it, the written one and its descendants, by class, while the move stays inside
/// the DataModel ([`REPARENTED`]). A playing client (roblox-sim, Studio 0.747, headless and rendered; os.clock around single moves, the
/// median of 15; docs/engine.md "Moves and ancestor reads") moved Models and Folders of 10 to 10,000 instances between two Folders of
/// ReplicatedStorage, of the Workspace, and between ReplicatedStorage and Lighting: 0.066 to 0.093 us an instance (Parts, Models,
/// Folders, values; a part's Attachment, Decal, Texture, ParticleEmitter or PointLight 0.072 to 0.087), a ScreenGui's Frames,
/// ImageLabels and TextLabels 1.84 to 2.16, a part's SurfaceGui 0.48 and Sound 0.63. Moved between two parentless Folders, 0.047 an
/// instance (Parts and Folders alike: [`REPARENTED_LOOSE_INSTANCE`]). These are a flat subtree's, 2 ancestors deep (a Folder of a
/// service): an instance pays [`REPARENTED_KEPT`] more for each ancestor the move keeps past 2, and [`REPARENTED_NESTED`] (outside the
/// DataModel [`REPARENTED_LOOSE_NESTED`], into or out of it [`MOVED_NESTED`]) for each level it sits below the moved one past the
/// first.
pub fn reparented(reflection: &Reflection, class: ClassId) -> f64 {
    if reflection.is_a(class, "GuiObject") {
        REPARENTED_GUI_OBJECT
    } else if reflection.is_a(class, "LayerCollector") {
        REPARENTED_LAYER_COLLECTOR
    } else if reflection.is_a(class, "Sound") {
        REPARENTED_SOUND
    } else {
        REPARENTED_INSTANCE
    }
}
pub const REPARENTED_INSTANCE: f64 = 0.075e-6;
pub const REPARENTED_GUI_OBJECT: f64 = 1.85e-6;
pub const REPARENTED_LAYER_COLLECTOR: f64 = 0.48e-6;
pub const REPARENTED_SOUND: f64 = 0.63e-6;
pub const REPARENTED_LOOSE_INSTANCE: f64 = 0.047e-6;
/// How a carried instance's price grows with its depth ([`reparented`]). The same client moved a Folder of 1,000 Folders between two
/// Folders at the bottom of a chain D deep in ReplicatedStorage: 69 us at D = 1, 1,858 at 300, 6,001 at 1,000, so 6.0 ns an instance
/// for each ancestor kept. A chain of N Folders (each in the one before) moved between two Folders of ReplicatedStorage: 22 us at
/// N = 100, 99 at 300, 855 at 1,000 (a flat Folder of 1,000: 70), so 1.6 ns for each level an instance sits below the moved one;
/// between two parentless Folders 17, 93 and 801 (1.5 ns); into and out of the DataModel 42 to 56, 364 to 408, 3,800 to 3,990
/// (7.8 ns). A chain of Parts (each in the one before) costs more a level: 40, 272 and 3,052 us inside the DataModel (6.0 ns), 36, 271
/// and 3,255 outside it (6.4), and into and out of it 65 to 86, 532 to 600, 6,171 to 6,485 (10.8 beyond the parts' own price; into the
/// Workspace 11.1 ms at 1,000, which is not modelled). A 10-ary tree of 1,111 Folders, 3 levels, read 75.5 us against a flat Folder's
/// 70.6. The client's Destroy of a chain grows with the cube of its depth (296 ms at 1,000, 7.8 s at 3,000), as each instance it takes
/// out after the first carries the rest of the chain out of a parentless tree: ree's charges it the same way.
pub const REPARENTED_KEPT: f64 = 6.0e-9;
pub const REPARENTED_NESTED: [f64; 2] = [1.6e-9, 6.0e-9];
pub const REPARENTED_LOOSE_NESTED: [f64; 2] = [1.5e-9, 6.4e-9];
pub const MOVED_NESTED: [f64; 2] = [7.8e-9, 10.8e-9];
/// What `levels` cost at `prices` ([`REPARENTED_NESTED`] and the others): those of instances other than parts, and those of parts.
pub fn nested(prices: [f64; 2], levels: [usize; 2]) -> f64 {
    levels[0] as f64 * prices[0] + levels[1] as f64 * prices[1]
}
/// A part whose world (its nearest WorldRoot ancestor) a Parent change inside the DataModel changes, for each world it leaves and each
/// it joins, beyond [`reparented`]: the same client's Models of 100 to 10,000 parts (spread out or stacked, in Models of 10 or not)
/// between ReplicatedStorage and the Workspace 0.96 to 1.12 us a part at 1,000 (to 1.3 headless, 2.2 to 2.6 rendered at 10,000),
/// into a WorldModel in ReplicatedStorage 0.71 to 0.92, between two such WorldModels 1.39 to 1.42, between the Workspace and a
/// WorldModel in it 1.54 to 1.66: the Workspace's world 0.90 us, a WorldModel's 0.70.
pub const WORLD_WORKSPACE_PART: f64 = 0.90e-6;
pub const WORLD_MODEL_PART: f64 = 0.70e-6;
/// An instance a Parent change inside the DataModel takes into or out of the Workspace's subtree (what a client draws, a WorldModel
/// in it included), beyond [`reparented`] and the worlds: a part 0.20 us (a rendered client's move into the Workspace 0.14 to 0.66
/// dearer than out of it, a headless client's alike), another instance 0.10 (Folders, values, Models and a part's children 0.07 to
/// 0.15 headless, 0.10 to 0.33 rendered).
pub const PLACED_PART: f64 = 0.20e-6;
pub const PLACED_OTHER: f64 = 0.10e-6;
/// What a Parent change that takes parts or joints into a world spends on their joints (world/joints.rs entering_price), beyond the
/// instances it carries: each joint in a world it looks at ([`JOINT_LOOKED_AT`]: those in the lists of the parts it carries, and
/// each joint it carries or that links a part it carries, once at its part that stays), each link it makes ([`JOINT_LINKED`]) and
/// each joint it brings into the world itself ([`JOINT_ENTERED`]); and, when a link it makes reaches a part that was in the world
/// already, a link change's price ([`LINK_CHANGED`] and the rest). A playing client (roblox-sim, Studio 0.747, headless; os.clock around single moves of a Part named by N
/// Welds into the Workspace and into a WorldModel in ReplicatedStorage, from nil and from ReplicatedStorage, the median of 11 at a
/// frame's start, N = 1,000 and 10,000, less the same move of a Part with none; docs/engine.md "Rigid assemblies") spent 0.043 to
/// 0.089 us a weld in a world that could not link (disabled, Part1 nil, its other part outside every world or in another world, in
/// JointsService), 0.08 to 0.09 us a part of an assembly of N it joined, 0.58 to 0.99 us a weld that linked it to an anchored part,
/// and 0.33 to 0.63 us a weld carried in under it that could not link; a weld outside every world cost it nothing, and the walks
/// do not look at those. Fitted: x0.56 to x1.61 of each case, x0.96 to x1.0 for 10,000 welds that link. Not modelled: a link that joins unanchored
/// parts into one assembly cost that client 2.9 to 8.5 us a weld at N = 1,000 and 10,000, a weld carried in that links 3.2 to 7.3,
/// and a Model of unanchored parts welded in a chain or a star grows with the square of its parts (14 to 18 us a part at 100, 92
/// to 100 at 1,000, a chain 951 at 10,000); all anchored, 1.3 us a weld (charged 1.0).
pub const JOINT_LOOKED_AT: f64 = 0.05e-6;
pub const JOINT_LINKED: f64 = 0.6e-6;
pub const JOINT_ENTERED: f64 = 0.3e-6;
/// What a Parent change's joints cost ([`JOINT_LOOKED_AT`]): `looked` joints looked at, `made` links it made and `entered` joints it
/// brought into the world.
pub fn joints_entering(looked: usize, made: usize, entered: usize) -> f64 {
    looked as f64 * JOINT_LOOKED_AT + made as f64 * JOINT_LINKED + entered as f64 * JOINT_ENTERED
}
/// What moving an assembly spends on its other parts and joints (world/joints.rs part_moved, end_batch), beyond the write: each part a
/// CFrame write (a script's, a tween's, BulkMoveTo's) or a PivotTo / ScaleTo carries with the part it wrote through their joints
/// ([`JOINT_CARRIED`]), and each joint between the moved assembly and another ([`JOINT_ACROSS`]; BulkMoveTo's moves do not pay it).
/// A playing client (roblox-sim, Studio 0.747, headless; os.clock around single writes, the median of 15 samples of 10 to 20,000,
/// less an empty loop; docs/engine.md "Rigid assemblies", 2026-11-24): an anchored part welded to N = 100 to 10,000 unanchored parts
/// moved by a CFrame write, by PivotTo of a Model holding it, by BulkMoveTo (either mode), or one of those parts moved, 0.077 to 0.081
/// us a part beyond the 1.0 to 1.3 of a part with none (one run read 0.09 to 0.21 at 10,000); an unanchored part welded to N anchored
/// parts (it joins one of their assemblies), 1.8 ns a weld by a CFrame write or PivotTo, nothing by BulkMoveTo. Welds that cannot
/// link (Part1 nil, disabled) cost nothing.
pub const JOINT_CARRIED: f64 = 0.08e-6;
pub const JOINT_ACROSS: f64 = 1.8e-9;
/// What a move spends on its assembly ([`JOINT_CARRIED`]): `carried` parts placed through their joints, `across` joints to other
/// assemblies.
pub fn assembly_moved(carried: usize, across: usize) -> f64 {
    carried as f64 * JOINT_CARRIED + across as f64 * JOINT_ACROSS
}
/// What a Position, Orientation or Rotation write spends rewriting the joints of the part it moves alone (world/joints.rs move_alone),
/// beyond the write: each joint whose C0, C1 or offset it rewrites ([`JOINT_REWRITTEN`]), and for each of those inside the part's own
/// assembly, each part of that assembly ([`JOINT_REWRITTEN_PART`]: the client settles the whole assembly again at each). The same
/// client: an anchored part welded to N unanchored parts, a Position write 242 us at N = 100, 1,944 at 300, 20,523 at 1,000 and 192,244
/// at 3,000 (0.0205 to 0.0242 us N squared; an Orientation write the same, 19,911 at 1,000); an unanchored part welded to N anchored
/// parts 4.7, 40 and 147 us at 100, 1,000 and 3,000 (0.036 to 0.049 us a weld). Fitted: x0.98 to x1.11 of the first, x0.82 to x1.09
/// of the second.
pub const JOINT_REWRITTEN: f64 = 0.04e-6;
pub const JOINT_REWRITTEN_PART: f64 = 0.021e-6;
/// What a move of a part alone spends on its joints ([`JOINT_REWRITTEN`]): `rewritten` joints, `inside` of them inside its assembly of
/// `parts`.
pub fn joints_rewritten(rewritten: usize, inside: usize, parts: usize) -> f64 {
    rewritten as f64 * JOINT_REWRITTEN + inside as f64 * parts as f64 * JOINT_REWRITTEN_PART
}
/// What an Anchored write that changes a part's assemblies spends (world/joints.rs reanchored), beyond the write: each part whose
/// assembly root or the root's anchoring it changes ([`JOINT_REANCHORED`]) and each joint linking the part ([`JOINT_ANCHOR_LINK`]).
/// The same client: an anchored part welded to N unanchored parts unanchored and anchored again, 77 us at N = 100, 130 at 300, 406 at
/// 1,000, 1,544 at 3,000 and 10,741 at 10,000 (each of the N + 1 parts' assembly changes); an unanchored part welded to N anchored
/// parts, 3.1, 15, 42 and 144 us at 100 to 10,000 (it alone changes: 0.014 us a weld); one of the hub's welded parts 1.7 to 1.8 us at
/// any N, as a part with none; a part named by welds that cannot link 1.6. Fitted: x0.61 at 100, x0.86 to x1.10 from 300 to 3,000,
/// x0.41 at 10,000 (the client's per-part cost doubles there), x0.98 to x1.08 of the second.
pub const JOINT_REANCHORED: f64 = 0.43e-6;
pub const JOINT_ANCHOR_LINK: f64 = 0.014e-6;
/// What an Anchored write spends on assemblies ([`JOINT_REANCHORED`]): `changed` parts, `links` joints linking the part.
pub fn joints_reanchored(changed: usize, links: usize) -> f64 {
    changed as f64 * JOINT_REANCHORED + links as f64 * JOINT_ANCHOR_LINK
}
/// What a link change costs (world/joints.rs charge_link), beyond the write: an Enabled write that links or unlinks a joint's two
/// parts, a Part0 or Part1 write that re-points a joint that links, a Parent write or Destroy that takes a linking joint or part
/// out of its world, a Parent write that takes one into a world where it links a part that was there, or an Anchored write of a
/// part that links others. A playing client (roblox-sim, Studio 0.747, headless; os.clock around
/// single writes at a frame's start and in turn after it, the median of 9 to 11 samples, an anchored or unanchored part welded to N
/// = 1 to 10,000 parts and chains of them; docs/engine.md "Rigid assemblies", 2026-11-24) spends 31 to 66 us on a frame's first
/// link change ([`LINK_FRAME`] beyond the rest), 6 to 20 us on the first change of another assembly in that frame
/// ([`LINK_ASSEMBLY`] beyond a change's own), each time 0.085 us a part of the assembly ([`LINK_PART`]: 871 to 921 us at 10,000
/// welded to a hub, 69 to 131 at 1,000, a leaf's weld disabled or enabled or the leaf anchored or not), and 1.8 to 3.9 us on each
/// later change of an assembly changed in the frame, whatever its size ([`LINK_CHANGED`] beyond the write's own price; two of them
/// cost more: [`LINK_REJOINED`]). The other
/// kinds cost it alike (2026-11-24, a leaf of an anchored hub welded to 10,000 parts, at a frame's start: its weld's Part1
/// re-pointed 874 us, set to nil 885, its Part0 re-pointed 926, the weld's Parent set to nil 864, the weld destroyed 883, the
/// leaf's Parent set to nil 982, the leaf destroyed 929, beyond what such a write or destroy costs alone; 10 to 43 us later in the
/// frame; its weld parented back under it 905 to 956, the leaf parented back 933, 17 to 30 later).
pub const LINK_CHANGED: f64 = 1.7e-6;
/// Two later link changes in a frame cost more, whichever assemblies they change (the same client, two to four sessions, single
/// writes in turn in a frame on stars of 100 to 10,000 parts and on two such stars interleaved; docs/engine.md "Rigid assemblies",
/// 2026-11-24): the frame's first that starts a link after one ended one ([`LINK_REJOINED`]: a weld enabled after one was disabled
/// 7.6 to 14.4 us, 8.5 to 9.4 at 1,000, where the later ones cost 1.8 to 2.9), and its second that ends one before any starts one
/// ([`LINK_SPLIT_AGAIN`]: 4.0 to 8.7 us, 4.4 to 4.6 at 1,000). Fitted: x0.63 to x1.18 of the first, x0.53 to x1.15 of the second.
pub const LINK_REJOINED: f64 = 7e-6;
pub const LINK_SPLIT_AGAIN: f64 = 2.6e-6;
pub const LINK_FRAME: f64 = 34e-6;
pub const LINK_ASSEMBLY: f64 = 5e-6;
pub const LINK_PART: f64 = 0.085e-6;
/// What the first link change of an assembly in a frame costs it ([`LINK_ASSEMBLY`]): `parts` parts.
pub fn link_assembly(parts: usize) -> f64 {
    LINK_ASSEMBLY + parts as f64 * LINK_PART
}
/// Each part of an assembly an AssemblyCenterOfMass read (an AssemblyMass read too, when none of its parts is anchored) works out
/// after a link change in the same frame (world/joints.rs pay_body): the same client read the centre at 0.08 us a part of the hub's
/// assembly after each link change (836 us at 10,000, 82 at 1,000, 12 at 100; a mass read of an anchored hub's stays flat, an
/// unanchored hub's mass read works out both: 621 to 947 us at 10,000), and the same read again with no change between at 0.03 to
/// 0.08 us, as before.
pub const BODY_REBUILT_PART: f64 = 0.08e-6;
/// Each part of the assembly an AssemblyLinearVelocity, AssemblyAngularVelocity, Velocity or RotVelocity write sets (world/gravity.rs
/// assign), beyond the write: the same client, the anchored part welded to N unanchored parts, 1.17 us at N = 100, 4.97 at 1,000, 13.2
/// at 3,000 and 45.9 at 10,000 (AssemblyAngularVelocity 4.84 and 43.9), beyond the 0.45 to 0.57 of a part with none; welded to N
/// anchored parts, the same as none.
pub const ASSEMBLY_VELOCITY_PART: f64 = 4.5e-9;
/// An ancestor IsDescendantOf, IsAncestorOf and the FindFirstAncestor finds look at, beyond [`method`]'s price for the call: the same
/// client called them on a Part at the bottom of a chain of D Folders in the Workspace (os.clock around 5 to 20,000 calls, the median of
/// 15, less an empty loop): 0.046 to 0.075 us a call at D = 1 and 10, 0.12 to 0.16 at 100, 1.13 to 1.34 at 1,000, 5.1 to 6.4 at 4,000,
/// whether the call finds its ancestor at the top or walks to the DataModel, the Workspace included (a script's call walks the parents
/// as a client's does, where the engine's own checks, its own Luau's calls among them, read the homes the dom keeps).
pub const ANCESTOR_STEP: f64 = 1.2e-9;
/// A change to a part below WorldModels, beyond its price, for each WorldModel above it and each pair of them: the same client
/// (os.clock around 10 to 4,500 writes, the median of 15, less an empty loop) changed a Part at the bottom of L WorldModels
/// nested one in another, the outermost in the Workspace or unparented, built from the top down, and spent this much more than
/// on the same part with none (a Transparency write): 0.02 us at L = 1, 0.30 at 10, 1.07 at 30, 7.1 to 9.4 at 100, 64 to 114 at
/// 300, 0.80 to 1.0 ms at 1,000, 7.8 to 8.2 ms at 3,000 (a 10,000-deep chain in the Workspace did not finish in 300 s); each
/// WorldModel above looks again at those above it. Fitted on 142 cases of 1 to 3,000 levels and 14 write kinds, each case's
/// extra over its changes ([`part_changes`]): x0.94 at the median, x0.65 to x1.53 from the 10th to the 90th percentile, x1.6
/// to x2.3 out of the Workspace at 10 to 100 levels. A plain Model above a part costs it about as much (7.1 us at 100, 41 to
/// 46 at 300: not modelled, the dom counts the WorldRoots), and a Folder 12 ns a level (not modelled either).
pub const WORLD_LEVEL: f64 = 13e-9;
pub const WORLD_LEVEL_PAIR: f64 = 1.6e-9;
/// What `changes` changes of a part below `levels` WorldModels cost for them ([`WORLD_LEVEL`]): nothing below none.
pub fn world_levels(levels: u32, changes: u32) -> f64 {
    let levels = f64::from(levels);
    f64::from(changes) * (levels * WORLD_LEVEL + levels * (levels + 1.0) / 2.0 * WORLD_LEVEL_PAIR)
}
/// How many changes a client makes of a part when `property` changes, each paying the WorldModels above it once
/// ([`world_levels`]): measured against Transparency below 300 and 1,000 WorldModels, CFrame, Position, Orientation (and so
/// Rotation) and a part's PivotTo 5 (4.75 to 5.1), Size 4 (3.9 to 4.0), Anchored, Color and BrickColor 3 (2.85 to 3.08),
/// Material, CanCollide, Reflectance, Name and Parent 1 (0.92 to 1.18), as is each part a Model's PivotTo moves (0.97 to 1.04 a
/// part of 10). Another property counts 1. CollisionGroup counts 2, and 3 when the write changed the part's CollisionGroupId
/// (`id_changed`): a playing client (Studio 0.748, two runs, the median of 15) wrote a name of another id below 100 and 300
/// WorldModels in the Workspace and in ReplicatedStorage at 2.66 to 3.46 changes, a name of the same id (two not registered) at
/// 1.84 to 2.18, and either below WorldModels in no parent, where it works no id out, at 2.02 to 2.08.
pub fn part_changes(property: &str, id_changed: bool) -> u32 {
    match property {
        "CFrame" | "Position" | "Orientation" | "Rotation" => 5,
        "Size" => 4,
        "Anchored" | "Color" | "BrickColor" => 3,
        "CollisionGroup" => if id_changed { 3 } else { 2 },
        _ => 1,
    }
}
/// GetFullName (and an error that names an instance by it) beyond the call: each name it joins and each byte of its text. The same
/// chains, named with 1, 6 or 60 bytes each: 4.46 us at 1,000 deep with 1-byte names, 5.34 with 6 bytes, 13.07 with 60; 18.96,
/// 22.50 and 52.24 at 4,000.
pub const FULL_NAME_NAME: f64 = 4.2e-9;
pub const FULL_NAME_BYTE: f64 = 0.14e-9;
/// What GetFullName's names and bytes cost ([`FULL_NAME_NAME`], [`FULL_NAME_BYTE`])
pub fn full_name(names: usize, bytes: usize) -> f64 {
    names as f64 * FULL_NAME_NAME + bytes as f64 * FULL_NAME_BYTE
}
/// The parallel bridge's machinery (an empty test place's client, an actor_cost probe, one session, 2026-10-23): BindableFunction:Invoke into an
/// Actor's OnInvoke 1,025 ns (ree's call itself counts 81.5), SharedTable.increment 370-429 (counts 31.5); a read (29 ns) is not priced beyond
/// the engine call ree's SharedTable makes of it but by its key's bytes, an integer key it finds and the value it makes ([`SHARED_KEY_BYTE`],
/// [`SHARED_FOUND_INTEGER`], [`SHARED_MADE_VALUE`] and after, [`SHARED_LOADED_BUFFER`]), a write is ([`SHARED_OVERWRITE_STRING`] and after).
/// Charged on top of what the instructions count.
pub const ACTOR_INVOKE: f64 = 0.94e-6;
pub const SHARED_INCREMENT: f64 = 0.37e-6;
/// SharedTable's calls by the entries they handle, beyond the call (lua/shared_table.rs), in seconds. A playing client (Studio 0.748,
/// os.clock over 10 to 10,000 entries, string and integer keys, two runs, fitted for the least relative error): SharedTable.new from a
/// table 0.32 us an entry with a string key and 0.19 with an integer key (a nested table's entries counted alike: 1,000 one-entry tables
/// took 0.71 ms; a table written to a key converts at the same price, and a conversion that fails pays for what it walked); a deep clone or cloneAndFreeze 10 and 6 ns an entry and 0.52 us a SharedTable it copies; a loop 0.4 us to start and
/// 37 and 29 ns an entry, of which the VM already charges each step's call (8 ns here), so 29 and 21 ns on top, charged at its start and at
/// each step (a loop stopped at its first entry took 0.24 to 0.29 us from 10 entries to 100,000, two runs, and 0.38 to 0.73 in another
/// probe at 3 to 200,000: Roblox walks a snapshot of its trie, not a list of its entries). A shallow clone, cloneAndFreeze, size and a
/// shallow clone's clear took 0.1 to 0.3 us at 10 entries as at 10,000 (Roblox shares the structure), so they cost only the call; a clear
/// of a table that alone holds its entries frees them ([`SHARED_CLEARED_ENTRY`]).
pub const SHARED_NEW_STRING_ENTRY: f64 = 0.32e-6;
pub const SHARED_NEW_INTEGER_ENTRY: f64 = 0.19e-6;
pub const SHARED_DEEP_STRING_ENTRY: f64 = 0.010e-6;
pub const SHARED_DEEP_INTEGER_ENTRY: f64 = 0.006e-6;
pub const SHARED_DEEP_TABLE: f64 = 0.52e-6;
pub const SHARED_LOOP: f64 = 0.4e-6;
pub const SHARED_LOOP_STRING_ENTRY: f64 = 0.029e-6;
pub const SHARED_LOOP_INTEGER_ENTRY: f64 = 0.021e-6;
/// A SharedTable write beyond the VM's call to it (lua/shared_table.rs `write_price`), in seconds, by its key's kind. A playing client (Studio
/// 0.748, os.clock over tables of 10 to 10,000 entries, two runs): an overwrite of a key the table has took 0.07-0.09 us with a string key and
/// 0.10-0.11 with an integer key; a key added took 0.31-0.45 and 0.19-0.23, one taken out (written nil) 0.23-0.39 and 0.15-0.23. Right after a
/// shallow clone a write copies the path it changes, which the clone shares: the first took 0.47 to 1.16 us with string keys and 0.55 to 1.10
/// with integer keys as the table grew from 10 entries to 10,000, the next ones as much again where their paths were still shared (a 10-entry
/// table's second write cost an overwrite). Priced as an overwrite or a key added or taken out, plus each node of ree's trie the write copied
/// because a copy shared it, fitted for the least relative error over the 64 measured cases of each kind (within x1.20 on average with string
/// keys, x1.25 with integer keys, whose trie copies seven small nodes where a client's cost grows with the table); the VM already charges
/// 0.02 us of each write's call.
pub const SHARED_OVERWRITE_STRING: f64 = 0.065e-6;
pub const SHARED_OVERWRITE_INTEGER: f64 = 0.10e-6;
pub const SHARED_CHANGE_STRING: f64 = 0.29e-6;
pub const SHARED_CHANGE_INTEGER: f64 = 0.17e-6;
pub const SHARED_COPIED_STRING_NODE: f64 = 0.24e-6;
pub const SHARED_COPIED_INTEGER_NODE: f64 = 0.065e-6;
/// Keys whose 64-bit hashes are equal, which a SharedTable keeps in one list past its trie's last level, newest first (lua/shared_table.rs
/// `find`), in seconds: an entry a search compared and a byte its comparison read, an entry of a list a key joined or left (rebuilt), and
/// an entry of a list a write copied because a shallow copy (or update's own copy) shared it. A playing client (Studio 0.748, real 64-bit
/// FNV-1a collisions, 208-byte keys at 256 to 8,192 a list, and 255 keys sharing a 64 KB prefix; one op on each of 20 clones): the oldest
/// key read 3.4 / 13.7 / 52.6 us at 256 / 1,024 / 4,096 keys (37.9 us at 8,192 read again and again), 557 to 892 us with the 64 KB
/// prefix; a key added 6.9 / 39.5 / 145.7 us, the oldest taken out 9.0 / 42.3 / 173.8 and the newest 7.1 / 30.3 / 126.1; the first write
/// after a clone 20 to 37 / 88 to 94 / 300 to 343 us whatever it wrote; update on its own list of 2,048 keys 290 to 306 us; Roblox's
/// search compares the keys byte by byte from the front. Fitted for the least relative error on the 41 rows (within x0.52 to 1.64; reads
/// of a list a client's cache holds come out dearer, of one it does not cheaper). A key added to a list of 8,192 so costs 0.3 ms, as a
/// client's 438 us does, and a list grows only as fast as a script can pay for it: ree sets no cap (a client kept 8,192 keys of one hash, the
/// most tried, so it has none up to there).
pub const SHARED_LIST_SCANNED: f64 = 5.2e-9;
pub const SHARED_COMPARED_BYTE: f64 = 0.031e-9;
pub const SHARED_LIST_REBUILT: f64 = 31e-9;
pub const SHARED_LIST_COPIED: f64 = 90e-9;
/// A string key's bytes, which a SharedTable call hashes once (a read, a write, increment, update, each string key a conversion walks, and
/// a key a slot split moves down, hashed again), and the bytes of a key a write or a conversion adds, which it copies in, in seconds a
/// byte. A playing client (Studio 0.748, os.clock, keys of 16 bytes to 4 MB, two and three sessions of each probe, the 8- or 16-byte row
/// of each call taken off): below 64 KB a read took 0.76 to 0.82 ns a byte, a hit and a miss and a key read again alike (Roblox hashes
/// the whole key on every call), an overwrite 0.78 to 0.84, a removal 0.89, increment 0.82 and update 0.81, a key added 1.14 and new(t)
/// 0.93 a key; a 3-byte key added and taken out beside a 1 MB key that shares its first slot took 793 us, beside one that does not 0.16.
/// Fitted for the least relative error on the 108 rows below 65,488 bytes, where a client's time steps (a key of 65,440 bytes read in 54
/// us, one of 65,488 in 83 to 98; within x0.79 to 1.19). From there a client takes longer, which these leave out (most likely a copy of
/// the key into a block of its own, inferred from the timings' shape, not observed): reads take x0.54 to 0.94 of it, writes x0.48 to 0.98,
/// increment x0.38 to 0.66 and update x0.26 to 0.53. An update whose callback is called again pays for its key again
/// (lua/shared_table.rs `update`).
pub const SHARED_KEY_BYTE: f64 = 0.81e-9;
pub const SHARED_KEY_ADDED_BYTE: f64 = 0.19e-9;
/// A string or buffer a SharedTable write stores (or update's result), in seconds: a byte copied below [`SHARED_LARGE_STRING`] bytes for a
/// string and [`SHARED_LARGE_BUFFER`] for a buffer, and from there a block and a byte (a client's time steps there, most likely as the
/// copy needs a block of its own, which is inferred from the timings, not observed: a string of 65,440 bytes written in 1.7 to 2.0 us, one
/// of 65,480 in 1.39 and one of 65,488 in 88 to 138; a buffer of 65,488 in 1.4 and one of 65,504 in 30 to 32; a key steps where a string
/// does, [`SHARED_KEY_BYTE`]). A playing client (Studio 0.748, os.clock, 16 bytes to 4 MB and a bisection of the step, three sessions): a
/// string 0.016 to 0.022 ns a byte below the step, 31 to 138 us at 64 KB and 322 to 781 at 1 MB; a buffer 0.011 below, 18 to 94 us at 64
/// KB and 165 to 556 at 1 MB (sessions up to 2.4x apart above the step). Fitted for the least relative error on the 156 rows (within x0.52
/// to 1.58 below the step, x0.36 to 2.93 from it).
pub const SHARED_COPIED_BYTE: f64 = 0.017e-9;
pub const SHARED_LARGE_STRING: usize = 65_488;
pub const SHARED_LARGE_BUFFER: usize = 65_504;
pub const SHARED_LARGE_COPY_BLOCK: f64 = 21.2e-6;
pub const SHARED_LARGE_COPY_BYTE: f64 = 0.44e-9;
/// A byte of a string or buffer a SharedTable conversion copies in (SharedTable.new, or a table written to a key), in seconds. A playing
/// client (Studio 0.748, new of 1 and of 16 entries of 16 bytes to 4 MB, two sessions): strings 0.17 ns a byte below 64 KB and 0.28 above,
/// buffers 0.16 and 0.12. Fitted for the least relative error on the 60 rows (within x0.31 to 3.1).
pub const SHARED_CONVERTED_BYTE: f64 = 0.25e-9;
/// A buffer a SharedTable read or a loop's step makes (a new buffer each time), in seconds: one of [`SHARED_LOADED_BUFFER_FROM`] bytes or
/// more (a block apart from Luau's pages), and a byte. A playing client (Studio 0.748, os.clock, 16 bytes to 4 MB, two and three
/// sessions): 48 ns at 16 bytes, 0.32 us at 256 and 2.4 to 3.5 at 1,024, 26 to 83 us at 64 KB and 279 to 371 at 1 MB; a loop's buffer
/// values alike. Fitted for the least relative error on the 86 rows of 1 KB and more (within x0.40 to 2.11).
pub const SHARED_LOADED_BUFFER: f64 = 2.5e-6;
pub const SHARED_LOADED_BUFFER_FROM: usize = 1024;
pub const SHARED_LOADED_BUFFER_BYTE: f64 = 0.47e-9;
/// A string a SharedTable read makes for a script (a read, update's old value) of [`SHARED_LARGE_STRING`] bytes or more, where a client's
/// reads step as its writes do, in seconds: a block and a byte. Below it the string is priced as the VM prices its own
/// ([`LUAU_STRING_BYTE`]: a client's reads 0.13 to 0.18 ns a byte) plus [`SHARED_MADE_STRING`], and a loop's keys and values at any size
/// as the VM prices them alone (a client's 0.13 ns a byte, with no step). A playing client (Studio 0.748, os.clock, 16 bytes to 4 MB and a
/// bisection of the step, two and three sessions): 65,440 bytes read in 11.5 us, 65,488 in 71 to 104, 128 KB in 50, 1 MB in 294 to 357.
/// Fitted for the least relative error on the 38 rows from the step (within x0.42 to 1.85; the 40 rows from 1 KB below it come within
/// x0.76 to 1.29 of the VM's price).
pub const SHARED_LOADED_STRING: f64 = 26.1e-6;
pub const SHARED_LOADED_STRING_BYTE: f64 = 0.27e-9;
/// A string a SharedTable read makes for a script below [`SHARED_LARGE_STRING`] bytes (a read, update's old value; not a loop's step), on
/// top of the VM's price a byte, in seconds. A playing client (Studio 0.748, os.clock, 100,000-read batches, two runs): a 16-byte string
/// read by the short string key "k5" took 0.063-0.069 us at 10 entries and 0.076-0.081 at 100,000, by the integer key 5 0.088-0.099 and
/// 0.088-0.091, 0.038 to 0.048 us more than the same reads of a number; a 1 KB string 0.20-0.21 us (ree 0.19 before this price, 0.23 with
/// it).
pub const SHARED_MADE_STRING: f64 = 0.04e-6;
/// The object a SharedTable read or update's old value makes for a script (a loop's step: [`SHARED_LOOP_MADE_VALUE`]), in seconds: a value type's userdata (CFrame,
/// Color3, UDim2, EnumItem, BrickColor, Faces, Axes, Ray, the keypoints, the int16 types, ...), one of the wider value types (TweenInfo,
/// Region3, PhysicalProperties and Path2DControlPoint, lua/shared_table.rs `WIDE`), and a nested SharedTable's new handle; a number, a
/// boolean and a native vector make none. A playing client (Studio 0.748, os.clock, medians of 7 batches of 50,000 reads of the key "v" of
/// a two-entry table, two runs, us a read): a number 0.0234-0.0239, a Vector3 0.0242-0.0247; BrickColor 0.031-0.036, Faces 0.031-0.037,
/// Axes 0.037, Ray 0.038-0.043, CFrame 0.038-0.053, Color3 0.039-0.040, EnumItem 0.039, NumberSequenceKeypoint 0.039-0.040, UDim2
/// 0.041-0.043; PhysicalProperties 0.045-0.046, TweenInfo 0.047-0.053, Path2DControlPoint 0.046-0.053, Region3 0.056-0.059; a SharedTable
/// 0.057 (0.047 in an earlier probe). Part of each median is a client's collector (the batches' minimums are 0.027-0.032 for the userdata
/// types), which these include. ree charged a number's price for each (0.019).
pub const SHARED_MADE_VALUE: f64 = 0.017e-6;
pub const SHARED_MADE_WIDE_VALUE: f64 = 0.03e-6;
pub const SHARED_MADE_HANDLE: f64 = 0.03e-6;
/// The object a SharedTable loop's step makes for a script, whatever its kind (a value type's userdata, a pooled value's copy, a
/// SharedTable's new handle), in seconds: a client's steps have no dearer kinds, as its reads do. A playing client (Studio 0.748,
/// os.clock, loops over 1,000 and 10,000 string-keyed entries, 20 to 200 a batch, two runs, us an entry beyond a loop over numbers,
/// 0.033 and 0.049): CFrame 0.024-0.026 at 1,000 and 0.023-0.026 at 10,000, Ray 0.019-0.020 and 0.011-0.020, TweenInfo
/// 0.022-0.023 and 0.020-0.023, Path2DControlPoint 0.020-0.022 and 0.018-0.025, a SharedTable 0.022 and 0.019-0.029. ree charged
/// a read's prices and, for a TweenInfo or a Path2DControlPoint, what the VM counted of the copy's own calls (0.012 to 0.015 more),
/// 0.017 to 0.045 in all.
pub const SHARED_LOOP_MADE_VALUE: f64 = 0.022e-6;
/// A SharedTable read by an integer key that finds a value, beyond a string key's, in seconds (update's old value too). A playing client
/// (Studio 0.748, os.clock, 100,000- and 50,000-read batches, two runs): a number or a boolean read by the key 5 took 0.049-0.052 us at 10
/// entries and at 100,000 alike, the key 500 of a 1,000-entry array 0.050-0.051, by the short string key "k5" 0.0245-0.0258 at 10 entries;
/// a missing key costs what a string key's miss does (0.019-0.029). ree charged 0.018.
pub const SHARED_FOUND_INTEGER: f64 = 0.03e-6;
/// An empty array slot or hash node of a table a SharedTable conversion walks, in seconds. A playing client (Studio 0.748, os.clock, two
/// runs): SharedTable.new of `table.create(n)` holding one entry took 19.9-20.0 us at n = 100,000 and 196-213 at 1,000,000 (0.20 to
/// 0.21 ns a slot), of `{x = 1}` 0.33-0.37. Fitted up to 1,000,000 slots. From there a client's time a slot grows, which this leaves out
/// (not matched): at 2,000,000 slots it took 529-533 us (0.27 ns a slot; ree 400, x0.75), and `table.create(4000000)` (4,000,000 slots)
/// took a client 4,634 us in an earlier session (1.16 ns a slot; ree 800). Only an array was measured, and an empty hash node is priced
/// as an array slot.
pub const SHARED_EMPTY_SLOT: f64 = 0.2e-9;
/// An entry SharedTable.clear frees, the table alone holding it, in seconds. A playing client (Studio 0.748, os.clock, two runs each,
/// tables of `k<i>` keys): at 1,000 entries 37-41 us, cleared right after new, a frame after it or built by writes (60 tables), and
/// 25-26 in a sub-millisecond batch (20 tables); at 10,000 205-218 us (3 tables), 260-279 (3 tables) and 262-268 (5 tables) in
/// sub-millisecond batches, 355-361 (10 tables) and 453-478 made 10 frames before (5 tables); a shallow clone's 0.02 to 0.77 us at any
/// size. 40 ns sits at the top of the warm rows and the bottom of the cold ones: ree's 40 and 400 us are 1.44 to 1.95x a client's in
/// the sub-millisecond batches, 0.98 to 1.13x in the 37-41 and 355-361 us rows, 0.84 to 0.88x in the 453-478 us one, and 0.48 to
/// 0.72x of an earlier session's 554-596 us at 10,000 and 7,676-8,397 at 100,000, most likely a colder state (10 entries took 0.25
/// and 0.27 us, 1.6x under it).
pub const SHARED_CLEARED_ENTRY: f64 = 40e-9;
/// TextService:GetTextSize (the parity bench: "Hello" and "World" at 14 in SourceSans 7.3 us on the fastest client sessions)
pub const TEXT_MEASURED: f64 = 7.3e-6;

/// A Text write on a TextLabel, TextButton or TextBox (a lobby client: 0.72-0.80 us with AutoLocalize off or digits only, 0.07
/// when the text is the one it has). With AutoLocalize on and letters in the text it also looks the text up for translation in a
/// place that has a localization table: [`text_localization_in`].
pub const TEXT_WRITE: f64 = 0.45e-6;

/// The places whose localization table Studio searches on a Text write (AutoLocalize on, letters in the text), by default
/// ([`PlacePrices::localized`]). Measured in a lobby's playtest client (the op_cost probe's `TextLabel.Text write`: 56.6-62.2 us
/// for a five-letter text in three sessions, 71.7-75.5 for forty in two, against 0.7-0.8 with AutoLocalize off). An empty test
/// place has none (1.0 us for the same write), so a place not listed pays nothing. A place_size_writes probe ("a" / "b" written to
/// a TextLabel in a playtest client, medians of five sessions) read 56-59 us a write in the same experience's match places (the
/// lobby 56 for the same one-letter text): a localization table belongs to the experience, not to a place, so every place of an
/// experience with one is listed. A test place's playtest client paid 32-47 us (its own table); its edit DataModel, which the
/// 1.0 us above was read in, pays none, and ree's place 0 stays priced as that. `(place id, us of the lookup for a one-letter
/// text)`: the lobby 56.1, the match places 61.2 (five sessions, median: [`LOCALIZATION_LOOKUP`]).
///
/// The parity bench (tools/cost-parity) reads "Hello" / "World" written with AutoLocalize on at 35.0 us in a rendered client of a game's
/// match place and 35.1-36.2 in a client of an empty place with no localization table, headless or rendered: a client looks every lettered
/// AutoLocalize text up, whatever the place holds, at 32.8 us for one letter ([`LOCALIZATION_LOOKUP`]) and, as the lobby measured,
/// 0.43 us a letter more. The Studio playtests' 56-61 us carried Studio's own text capture, and the 1.0 us of an empty place was its edit
/// DataModel's (no translator). So every client run pays it ([`text_localization_in`]); a place listed here pays its own figure.
pub const LOCALIZED_PLACES: &[(f64, f64)] = &[];

/// The lookup of a one-letter text in a client, in microseconds (above); `REE_LOCALIZED_PLACES <place id>=<us>` gives a place its own.
pub const LOCALIZATION_LOOKUP: f64 = 32.8;

/// The lookup a changed Text write adds in `place_id`: every client's (an edit DataModel and a server have no translator: None).
pub fn text_localization_in(prices: &PlacePrices, place_id: f64, characters: usize, client: bool) -> Option<f64> {
    if !client {
        return None;
    }
    let one_letter = prices.localized.iter().find(|(place, _)| *place == place_id && place_id != 0.0).map_or(LOCALIZATION_LOOKUP, |(_, us)| *us);
    Some((one_letter + 0.4286 * (characters as f64 - 1.0)) * 1e-6)
}

/// What a match place's client adds to a part's geometry write in the workspace, by default ([`PlacePrices::matched`]): a
/// place_size_writes probe in an empty test place's and a match place's playtest clients, 14 sessions of 7 repeats each:
/// `Part.CFrame = ...` 1.69-1.83 us in both at every part count from 0 to 20,000 (and the same with 6,000 Frames in a ScreenGui),
/// where an op_cost run read 1.67 in a match place and 1.10-1.15 in a lobby's and the test place's Edit DataModel; the same write
/// out of the DataModel is 0.16 in all of them, ree's price. A place's size does not move a price; its kind of DataModel does: a
/// playtest client pays 1.5x the Edit DataModel for a part that is in the workspace. A lobby's client reads 1.10 (an op_cost run)
/// and keeps ree's price, as does every place not listed. Position, Orientation and Rotation share CFrame's write path and are
/// scaled with it (not measured apart).
///
/// The parity bench measures the default prices in a match place's rendered client, and a headless client of that match place paid what
/// one of an empty place paid (CFrame 0.93 us against 0.88): no place is listed by default; `REE_MATCH_PLACES` still scales one.
pub const MATCH_PLACES: &[f64] = &[];
pub const MATCH_GEOMETRY_SCALE: f64 = 1.55;

/// The places whose clients pay more than ree's default prices ([`crate::Options::place_prices`]): those with a localization
/// table, whose Text writes pay the lookup ([`text_localization_in`]), and the match places, whose parts' geometry writes in the
/// workspace pay [`MATCH_GEOMETRY_SCALE`] ([`place_write_scale`]). The default is [`LOCALIZED_PLACES`] and [`MATCH_PLACES`];
/// a command line adds the places `REE_LOCALIZED_PLACES` and `REE_MATCH_PLACES` name ([`PlacePrices::with`]).
#[derive(Clone, Debug, PartialEq)]
pub struct PlacePrices {
    /// `(place id, us of the localization lookup for a one-letter text)`
    pub localized: Vec<(f64, f64)>,
    /// The match places' ids
    pub matched: Vec<f64>,
}

impl Default for PlacePrices {
    fn default() -> Self {
        Self { localized: LOCALIZED_PLACES.to_vec(), matched: MATCH_PLACES.to_vec() }
    }
}

impl PlacePrices {
    /// These prices with the places of a `REE_LOCALIZED_PLACES` value (`<place id>[=<us>],...`: the lookup of a one-letter
    /// text in microseconds, [`LOCALIZATION_LOOKUP`] when it is left out) and of a `REE_MATCH_PLACES` value (`<place id>,...`).
    /// A place listed already keeps its entry.
    pub fn with(mut self, localized: Option<&str>, matched: Option<&str>) -> Result<Self, String> {
        let number = |variable: &str, text: &str| text.trim().parse::<f64>().ok().filter(|n| n.is_finite()).ok_or_else(|| format!("{variable}: {text:?} is not a number"));
        for entry in localized.unwrap_or_default().split(',').filter(|entry| !entry.trim().is_empty()) {
            let (place, lookup) = match entry.split_once('=') {
                Some((place, lookup)) => (number("REE_LOCALIZED_PLACES", place)?, number("REE_LOCALIZED_PLACES", lookup)?),
                None => (number("REE_LOCALIZED_PLACES", entry)?, LOCALIZATION_LOOKUP),
            };
            if !self.localized.iter().any(|(known, _)| *known == place) {
                self.localized.push((place, lookup));
            }
        }
        for entry in matched.unwrap_or_default().split(',').filter(|entry| !entry.trim().is_empty()) {
            let place = number("REE_MATCH_PLACES", entry)?;
            if !self.matched.contains(&place) {
                self.matched.push(place);
            }
        }
        Ok(self)
    }
}

/// The factor a place puts on [`write`]'s price for `property` on a part in the workspace.
pub fn place_write_scale(prices: &PlacePrices, place_id: f64, property: &str, placed: bool) -> f64 {
    if placed && matches!(property, "CFrame" | "Position" | "Orientation" | "Rotation") && place_id != 0.0 && prices.matched.contains(&place_id) {
        MATCH_GEOMETRY_SCALE
    } else {
        1.0
    }
}

/// What the lookup adds to a changed Text write with AutoLocalize on and letters in the text, `characters` long (a line through
/// the five- and forty-character rows: 55.7 us and 0.43 us a character), in a place [`PlacePrices`] lists: [`text_localization_in`].
/// A property write by what the engine does for it, in the parity bench: a part's geometry in the
/// workspace is dear (CFrame 1.43 us, Position 1.42; Size 1.10, from Studio's ratio to CFrame: no row of its own outside the
/// held-out third), BrickColor 0.55, Material 0.38, Transparency 0.25, CanCollide / CastShadow / Reflectance 0.22-0.26; out of the
/// workspace geometry 0.145-0.165 and the rest 0.057; an effect's property (a Beam's, ParticleEmitter's, light's or Highlight's)
/// 0.17-0.19; a GUI object's 0.17-0.22, its Size 0.25, a Color3 0.32, an image 0.78, a Text 0.45; a UI modifier's (UIGradient,
/// UIStroke, UIScale) 0.16-0.24; an Attachment's 0.115; a joint's C0 0.14; a Camera's CFrame 0.34; Name 0.30; a Bone's or
/// Motor6D's Transform 0.06. A Studio playtest client, where these were measured before, read most of them 1.3-2.6x dearer and a
/// part's geometry 0.77x: its Explorer and Properties window listen, and only a rendered client pays for what is drawn. Nothing changed:
/// [`WRITE_QUIET`]. `placed` is whether the instance is in the workspace.
pub fn write(reflection: &Reflection, class: ClassId, property: &str, changed: bool, placed: bool) -> f64 {
    if !changed {
        return WRITE_QUIET;
    }
    if property == "Name" {
        return 0.30e-6;
    }
    let part = reflection.is_a(class, "BasePart");
    if part {
        return 1e-6
            * match (placed, property) {
                (true, "CFrame" | "Position" | "Orientation" | "Rotation" | "PivotOffset") => 1.43,
                (true, "Size") => 1.10,
                (true, "Color" | "BrickColor") => 0.55,
                (true, "Material") => 0.385,
                (true, "Transparency" | "LocalTransparencyModifier") => 0.25,
                (true, _) => 0.246,
                (false, "CFrame" | "Position" | "Orientation" | "Rotation" | "PivotOffset") => 0.165,
                (false, "Size") => 0.145,
                (false, _) => 0.057,
            };
    }
    if property == "Transform" && (reflection.is_a(class, "Motor6D") || reflection.is_a(class, "Bone")) {
        return 0.06e-6;
    }
    if (property == "C0" || property == "C1") && reflection.is_a(class, "JointInstance") {
        return 0.14e-6;
    }
    let name = &*reflection.class(class).name;
    if property == "Text" && matches!(name, "TextLabel" | "TextButton" | "TextBox") {
        return TEXT_WRITE;
    }
    if name == "Camera" && property == "CFrame" {
        return 0.345e-6;
    }
    if reflection.is_a(class, "Attachment") {
        return 0.115e-6;
    }
    if reflection.is_a(class, "GuiObject") {
        return 1e-6
            * match property {
                "Size" => 0.245,
                "Image" => 0.78,
                // (this property's price, which only reads like 1/pi: FRAC_1_PI is 0.31830988..., another number, and
                // the work clock's prices must not move, so the literal stays)
                #[allow(clippy::approx_constant)]
                "BackgroundColor3" | "BorderColor3" | "TextColor3" | "TextStrokeColor3" => 0.318,
                _ => 0.203,
            };
    }
    if reflection.is_a(class, "UIComponent") {
        return 1e-6 * if matches!(property, "Color" | "Transparency") { 0.218 } else { 0.20 };
    }
    if ["ParticleEmitter", "Beam", "Trail", "Light", "Highlight", "Fire", "Smoke", "Sparkles"].iter().any(|kind| reflection.is_a(class, kind)) {
        return 0.176e-6;
    }
    0.30e-6
}

/// Instance.new by class, in the parity bench: a Part 0.83 us, a MeshPart 1.89, a Model 0.58, a
/// Frame 1.07, an ImageLabel 0.65 and ImageButton 1.07, a TextButton 1.44, a ScreenGui 1.54, a BillboardGui 1.18, a UIListLayout 1.41,
/// a UIGradient 0.92, UIPadding and UIScale 0.72-0.78, a Motor6D 0.91, a Weld 0.52, and a Folder, Attachment, Bone, BindableEvent,
/// NumberValue, light, Highlight, Trail, Sparkles or WeldConstraint 0.32-0.38 (median 0.36). Humanoid, Animator and SurfaceAppearance keep a
/// Studio client's figures (no row).
pub fn created(reflection: &Reflection, class: ClassId) -> f64 {
    1e-6 * match &*reflection.class(class).name {
        "Humanoid" => 4.3,
        "TextLabel" | "TextButton" | "TextBox" => 1.60,
        "MeshPart" => 1.65,
        "ScreenGui" => 1.38,
        "BillboardGui" | "SurfaceGui" => 1.18,
        "Animator" => 1.6,
        "UIListLayout" | "UIGridLayout" | "UITableLayout" | "UIPageLayout" => 1.41,
        "ImageButton" => 0.84,
        "ImageLabel" => 0.65,
        "UIGradient" => 0.92,
        "Motor6D" => 0.91,
        "Model" => 0.52,
        "Weld" => 0.52,
        "SurfaceAppearance" => 1.2,
        _ if reflection.is_a(class, "BasePart") => 0.83,
        _ if reflection.is_a(class, "GuiObject") => 1.07,
        _ if reflection.is_a(class, "UIComponent") => 0.75,
        _ => 0.37,
    }
}

/// An instance copied by Clone, in the parity bench: a Part 3.46 us alone, 3.44 a part in a Folder or
/// Model of parts (the Folder or Model 0.48), a MeshPart 5.83, a ParticleEmitter 3.04, a TextLabel 4.69, a UIGradient 2.47, an Attachment
/// or Beam inside an effect's part 1.27; a GUI card's frames, image and UI modifiers are the old edit table's figures scaled to the
/// card's 21.9 us. Humanoid, Motor6D, Sound, ScrollingFrame and the Value, Constraint and light classes keep the edit place's figures.
pub fn cloned(reflection: &Reflection, class: ClassId) -> f64 {
    1e-6 * match &*reflection.class(class).name {
        "Humanoid" => 9.5,
        "TextLabel" | "TextButton" | "TextBox" => 4.69,
        "MeshPart" => 5.83,
        "ScrollingFrame" => 5.6,
        "ImageLabel" | "ImageButton" => 3.0,
        "ParticleEmitter" => 3.04,
        "UIGradient" => 2.47,
        "Model" | "Folder" => 0.48,
        "Motor6D" => 2.3,
        "Attachment" | "Beam" => 1.27,
        "UIListLayout" | "ScreenGui" | "CanvasGroup" | "BillboardGui" | "SurfaceGui" => 3.3,
        "Sound" => 2.5,
        _ if reflection.is_a(class, "BasePart") => 3.44,
        _ if reflection.is_a(class, "GuiObject") => 2.9,
        _ if reflection.is_a(class, "UIComponent") => 1.5,
        _ if reflection.is_a(class, "ValueBase") || reflection.is_a(class, "Constraint") || reflection.is_a(class, "Light") => 1.25,
        _ => 1.8,
    }
}
/// One vertex of an EditableMesh's GetVertices (capture: 0.065-0.104 us, 13,515 vertices)
pub const LISTED_VERTEX: f64 = 0.08e-6;

/// A named method's call, beyond what it lists.
pub fn method(name: &str) -> f64 {
    1e-6 * match name {
        // playtest: grass patches out of the DataModel 1.8 us; capture: a Part in Workspace 3.1-3.9 us
        // (Model:PivotTo, ScaleTo, GetPivot and the rest of the Luau-served model methods price themselves,
        // services.luau Priced; a Part's PivotTo too)
        "Fire" => 0.045 + FIRE * 1e6,
        // (the parity bench, beyond the Luau around the call: IsA 25 ns, GetFullName 102, GetPlayers 69, HasTag 41-43, GetTags of a
        // part with none 73, AddTag 393, RemoveTag 156 [single recorded row method.CollectionService.RemoveTag = 170.1 ns whole],
        // Emit(1) 383, Camera:WorldToViewportPoint 200, GetAttribute 38-40, TweenService:Create 1.31 us;
        // a ray and a bounds query by what they meet, [`RAY_MET`] and [`QUERY_BOX_PART`]; IsDescendantOf has rows only in the held-out
        // third: a Studio client's 0.015)
        "IsA" => 0.025,
        "IsDescendantOf" | "IsAncestorOf" => 0.015,
        "GetChildren" | "GetDescendants" => 0.039,
        "FindFirstChild" | "FindFirstChildOfClass" | "FindFirstChildWhichIsA" | "FindFirstAncestor" | "FindFirstAncestorOfClass" | "FindFirstAncestorWhichIsA" => 0.048,
        // (the bench's row, Workspace.ParityScene.Tree100.N1_1.N2_1, less its 5 names and 39 bytes: [`full_name`])
        "GetFullName" => 0.102 - (5.0 * FULL_NAME_NAME + 39.0 * FULL_NAME_BYTE) * 1e6,
        "GetPlayers" => 0.069,
        "HasTag" => 0.042,
        "GetTags" => 0.073,
        "AddTag" => 0.39,
        "RemoveTag" => 0.155,
        "Emit" => 0.29,
        "WorldToViewportPoint" | "WorldToScreenPoint" | "ViewportPointToRay" | "ScreenPointToRay" => 0.20,
        "GetAttribute" => 0.047,
        "Create" => 1.31,
        // (a miss with no filter: the bench's held-out 0.17 us, a headless 0.747 client 0.19 to 0.23; its miss with an Include
        // filter of one part, 0.27 here before, is this and the list's read, PARAMS_LIST: 0.31 to 0.33 on the same client)
        "Raycast" => 0.19,
        // (the bench's misses: a ray's, since PARAMS_LIST is priced separately; what they cross: SPHERE_CROSSED, BLOCK_CROSSED)
        "Blockcast" | "Spherecast" | "Shapecast" => 0.19,
        "GetPartBoundsInBox" | "GetPartBoundsInRadius" => 0.15,
        "GetPartsInPart" => 0.243,
        // (the Region3 queries and ArePartsTouchingOthers: an empty call in a playing client, less the 0.09 us its GetPartBoundsInBox took
        // over this table's 0.15 beside it (0.40, 0.42 and 0.27 us); what they find: [`REGION_FOUND`], [`TOUCHING_LIST`])
        "FindPartsInRegion3" | "FindPartsInRegion3WithIgnoreList" | "FindPartsInRegion3WithWhiteList" | "findPartsInRegion3" => 0.31,
        "IsRegion3Empty" | "IsRegion3EmptyWithIgnoreList" => 0.33,
        "ArePartsTouchingOthers" => 0.18,
        // (HttpService:JSONEncode and JSONDecode: the call, and by what they handle [`json`])
        "JSONEncode" | "JSONDecode" => JSON_CALL * 1e6,
        // (TweenService:GetValue of a linear alpha 51.8 ns whole, 27 beyond the Luau around it)
        "GetValue" => 0.027,
        // ReflectionService:GetClass's lookup; the entry it makes is [`REFLECTED_CLASS`]'s
        "GetClass" => 0.28,
        // the Luau-served model queries add their own MODEL_QUERY
        "GetPivot" | "GetScale" | "GetBoundingBox" | "GetExtentsSize" | "PivotTo" | "ScaleTo" => 0.0,
        // EditableMesh (capture, a 13,515-vertex mesh; playtest the two weights calls 0.84 us)
        "GetVertexBoneWeights" => 0.44,
        "GetVertexBones" => 0.55,
        "GetFaceVertices" => 0.3,
        "GetFaceColors" => 0.22,
        "GetFaceNormals" | "GetFaceUVs" => 0.22,
        "GetPosition" | "GetNormal" | "GetUV" => 0.33,
        "GetBoneName" => 1.0,
        "GetColor" | "GetColorAlpha" => 0.09,
        "SetColorAlpha" | "SetColor" => 0.1,
        "SetFaceColors" | "SetFaceNormals" | "SetFaceUVs" => 0.15,
        "SetPosition" | "SetNormal" | "SetUV" => 0.15,
        "RemoveFace" => 0.29,
        _ => return CALL,
    }
}

/// Where [`method`]'s price for `name` came from ([`kind`] bits): a method the table does not name borrows [`CALL`] and Shapecast the
/// Blockcast's (defaulted); EditableMesh's methods, IsDescendantOf / IsAncestorOf and the Luau-served model queries were measured in a
/// Studio capture or playtest (Studio-era); the Region3 queries (FindPartsInRegion3, its WithIgnoreList / WithWhiteList variants and
/// findPartsInRegion3, IsRegion3Empty, IsRegion3EmptyWithIgnoreList) and ArePartsTouchingOthers were timed with os.clock in a Studio
/// 0.748 playing client (world/region.rs: [`REGION_FOUND`], [`TOUCHING_LIST`]); the rest in the parity bench's client sessions.
pub fn method_kinds(name: &str) -> u8 {
    match name {
        "IsA" | "GetChildren" | "GetDescendants" | "FindFirstChild" | "FindFirstChildOfClass" | "FindFirstChildWhichIsA" | "FindFirstAncestor"
        | "FindFirstAncestorOfClass" | "FindFirstAncestorWhichIsA" | "GetFullName" | "GetPlayers" | "HasTag" | "GetTags" | "AddTag" | "RemoveTag" | "Emit"
        | "WorldToViewportPoint" | "WorldToScreenPoint" | "ViewportPointToRay" | "ScreenPointToRay" | "GetAttribute" | "Create" | "Raycast"
        | "Blockcast" | "Spherecast" | "GetPartBoundsInBox" | "GetPartBoundsInRadius" | "GetPartsInPart" | "JSONEncode" | "JSONDecode"
        | "GetValue" | "GetClass" | "Fire" | "GetPivot" | "GetScale" | "GetBoundingBox" | "GetExtentsSize" | "PivotTo" | "ScaleTo"
        | "FindPartsInRegion3" | "FindPartsInRegion3WithIgnoreList" | "FindPartsInRegion3WithWhiteList"
        | "findPartsInRegion3" | "IsRegion3Empty" | "IsRegion3EmptyWithIgnoreList" | "ArePartsTouchingOthers" => 0,
        "Shapecast" => kind::DEFAULTED,
        "IsDescendantOf" | "IsAncestorOf" | "GetVertexBoneWeights" | "GetVertexBones" | "GetFaceVertices" | "GetFaceColors" | "GetFaceNormals"
        | "GetFaceUVs" | "GetPosition" | "GetNormal" | "GetUV" | "GetBoneName" | "GetColor" | "GetColorAlpha" | "SetColorAlpha" | "SetColor"
        | "SetFaceColors" | "SetFaceNormals" | "SetFaceUVs" | "SetPosition" | "SetNormal" | "SetUV" | "RemoveFace" => kind::STUDIO,
        _ => kind::DEFAULTED,
    }
}

/// Where [`write`]'s price came from: a part's Size in the Workspace is Studio's ratio to CFrame applied to the bench's CFrame
/// (defaulted), and a property of a class the table does not name borrows 0.30 us (defaulted); a changed Text write with AutoLocalize is
/// the bench's; the rest the parity bench measured.
pub fn write_kinds(reflection: &Reflection, class: ClassId, property: &str, changed: bool, placed: bool) -> u8 {
    if !changed || property == "Name" {
        return 0;
    }
    if reflection.is_a(class, "BasePart") {
        return if placed && property == "Size" { kind::DEFAULTED } else { 0 };
    }
    let named = (property == "Transform" && (reflection.is_a(class, "Motor6D") || reflection.is_a(class, "Bone")))
        || ((property == "C0" || property == "C1") && reflection.is_a(class, "JointInstance"))
        || (property == "Text" && matches!(&*reflection.class(class).name, "TextLabel" | "TextButton" | "TextBox"))
        || (&*reflection.class(class).name == "Camera" && property == "CFrame")
        || reflection.is_a(class, "Attachment")
        || reflection.is_a(class, "GuiObject")
        || reflection.is_a(class, "UIComponent")
        || ["ParticleEmitter", "Beam", "Trail", "Light", "Highlight", "Fire", "Smoke", "Sparkles"].iter().any(|kind| reflection.is_a(class, kind));
    if named { 0 } else { kind::DEFAULTED }
}

/// Where [`created`]'s price came from: Humanoid, Animator and SurfaceAppearance keep a Studio client's figures (Studio-era)
pub fn created_kinds(reflection: &Reflection, class: ClassId) -> u8 {
    match &*reflection.class(class).name {
        "Humanoid" | "Animator" | "SurfaceAppearance" => kind::STUDIO,
        _ => 0,
    }
}

/// Where [`cloned`]'s price came from: Humanoid, Motor6D, Sound, ScrollingFrame, an image or a GUI card's parts, the layouts and the
/// Value, Constraint and light classes keep the edit place's figures (Studio-era); a class the table does not name borrows 1.8 us
/// (defaulted)
pub fn cloned_kinds(reflection: &Reflection, class: ClassId) -> u8 {
    match &*reflection.class(class).name {
        "TextLabel" | "TextButton" | "TextBox" | "MeshPart" | "ParticleEmitter" | "UIGradient" | "Model" | "Folder" | "Attachment" | "Beam" => 0,
        "Humanoid" | "ScrollingFrame" | "ImageLabel" | "ImageButton" | "Motor6D" | "UIListLayout" | "ScreenGui" | "CanvasGroup" | "BillboardGui" | "SurfaceGui"
        | "Sound" => kind::STUDIO,
        _ if reflection.is_a(class, "BasePart") => 0,
        _ if reflection.is_a(class, "GuiObject") || reflection.is_a(class, "UIComponent") => kind::STUDIO,
        _ if reflection.is_a(class, "ValueBase") || reflection.is_a(class, "Constraint") || reflection.is_a(class, "Light") => kind::STUDIO,
        _ => kind::DEFAULTED,
    }
}

/// A subtree entering or leaving the DataModel: [`MOVE_BASE`] once and an instance of it by class, in the parity bench
/// (a move in and a move out, averaged): a Folder 0.52 us, a Folder of 10 NumberValues 0.96 (so 0.044 an instance), a Part 1.47, a
/// Model of 3 parts 4.12 (so 1.10 a part, the lone Part 1.0), a Frame into a ScreenGui 1.17, an Attachment into a part 0.58. A Studio
/// capture read these 5-60x dearer (2.7-14.0 us an instance): its Explorer adds and removes a row for each. An unanchored part
/// resting on others costs far more to take out (capture: 0.5-1.3 ms), which is not modelled. The move's ancestor chains are priced
/// apart ([`reparent_chains`]), so [`MOVE_BASE`] is the bench's 0.47 us less what the bench's own chains cost (nil to a Folder 4
/// ancestors deep in the workspace and back, averaged).
pub fn moved(reflection: &Reflection, class: ClassId) -> f64 {
    1e-6 * if reflection.is_a(class, "BasePart") {
        1.20
    } else if reflection.is_a(class, "GuiObject") {
        0.78
    } else if reflection.is_a(class, "Attachment") {
        0.12
    } else {
        0.055
    }
}
pub const MOVE_BASE: f64 = 0.50e-6 - (reparent_chains(0, 4, 0) + reparent_chains(4, 0, 0)) / 2.0;

/// The client the prices were measured in, as the profile's headline names it: the parity bench's sessions (tools/cost-parity,
/// reference/), a roblox-sim client (Studio 0.744's engine, and 0.747's since string.split's refit) rendering a game's match place on one
/// desktop PC, not Studio
pub const REFERENCE_CLIENT: &str = "a rendered roblox-sim client (Studio 0.744 and 0.747 engines) of a game's match place, one desktop PC";

/// A component's band in [`BENCH_BANDS`]
pub struct BenchBand {
    pub component: &'static str,
    /// The rows it was measured on
    pub rows: usize,
    /// ree / client at the 10th and the 90th percentile (the range, for Studio-era)
    pub low: f64,
    pub high: f64,
}

/// ree / client by component of the work clock, on rows no fit saw ([`BenchBand`]: rows, 10th percentile, 90th percentile). Luau, engine,
/// dispatch and collector are the parity bench's held-out rows (a third of the corpus by a hash of the names; each row against the
/// fastest of the reference sessions in tools/cost-parity/reference): Luau the language and library rows the clock prices (lang, native,
/// lib, libsize, loop; the large buffer rows, which count bytes it prices at nothing, are left out, as a profile prints those bytes as
/// unpriced: [`LUAU_BUFFER_FRESH_BYTE`]), engine the engine calls' families (read, write, method, find, attr, create, clone, parent, destroy, tween, datatype, json),
/// dispatch the signal and task rows, collector the churn rows (gc). Cold is not a test of the cold charges alone: it is the macro
/// bench's 13 held-out rows (notes/macro: nine attacks' casts, a card grid and a server loop), the whole work clock on a game's code
/// where cold charges weigh most, against a client's script time, measured with the prices before the size fit. Studio-era is the
/// Studio playtest client's prices against the bench's interpreted rows (their range: the only rows that test a Studio-era price), and
/// defaulted what a borrowed price reads against what the bench measured for the calls it would stand in for (the 55 method rows at
/// CALL's 58 ns). A profile weighs them by its own work (profiler/report.rs fidelity_json). Luau, engine, dispatch and collector are
/// read against the 0.744 and the 0.747 sessions since string.split's refit (bands.py --priced; against the 0.744 sessions alone they
/// were 0.52-1.38 on 69 rows, 0.58-1.27, 0.83-1.66 and 0.61-1.04: the 0.747 sessions, faster on many rows, moved them, not the price).
pub const BENCH_BANDS: &[BenchBand] = &[
    BenchBand { component: "luau", rows: 78, low: 0.51, high: 1.42 },
    BenchBand { component: "engine", rows: 94, low: 0.67, high: 1.29 },
    BenchBand { component: "collector", rows: 4, low: 0.70, high: 0.96 },
    BenchBand { component: "cold", rows: 13, low: 0.53, high: 1.06 },
    BenchBand { component: "dispatch", rows: 6, low: 0.88, high: 1.92 },
    BenchBand { component: "studio", rows: 35, low: 0.45, high: 2.1 },
    BenchBand { component: "defaulted", rows: 55, low: 0.01, high: 1.42 },
];
/// ree / client over all 182 held-out rows of the parity bench (against the fastest of the 0.744 and 0.747 sessions, 10th to 90th
/// percentile; median |err| 16.5 %, median |ln(ree/client)| 15.8 %, p90/p10 2.14x, including all 7 held-out large buffer rows now priced
/// across cache tiers; previously 0.53-1.36x and 17.6 % with large buffers unpriced, or 0.60-1.38x on the 175 non-buffer rows).
pub const FIDELITY_BAND: (f64, f64) = (0.63, 1.35);
/// The phone factor's measured spread ([`PHONE_SCRIPTS`]): 2.13-4.16 over the phones and scenes compared
pub const PHONE_SCRIPTS_BAND: (f64, f64) = (2.13, 4.16);

/// What a device's client takes for a script's work, against the desktop prices above (`--device phone`): a conversion, not a measurement.
/// Every price in this file is a desktop client's (the parity bench's client, a Studio 0.744 engine on a desktop CPU); a phone's is that
/// times a sim-to-phone calibration of script time (phones' captured script milliseconds against the same scenes' desktop client: 2.77,
/// ranging 2.13-4.16 over the phones and scenes compared; weighted to live frames, 1.94). Its GUI figure (4.09) is the engine's layout
/// and drawing, which runs outside a script's os.clock and which the work clock does not charge.
pub const PHONE_SCRIPTS: f64 = 2.77;

/// The factor the work clock scales script work by for `device` (Options::device): [`PHONE_SCRIPTS`] for a phone, else 1.
pub fn device_scale(device: &str) -> f64 {
    if device == "phone" { PHONE_SCRIPTS } else { 1.0 }
}

/// An EditableMesh's own work, per vertex of the mesh, none of it yielding: RemoveUnused
/// (capture: 1.07-1.36 ms on 13,515 vertices), CreateMeshPartAsync with Box collision (playtest:
/// 1.9 ms on 13,515; capture 0.1 ms on 1,487) and CreateEditableMeshAsync of an EditableMesh's
/// Content (capture: 0.6 ms on 1,487).
pub const REMOVE_UNUSED_VERTEX: f64 = 0.09e-6;
pub const MESH_PART_VERTEX: f64 = 0.12e-6;
pub const MESH_COPY_VERTEX: f64 = 0.4e-6;

/// CreateEditableMeshAsync of a mesh asset yields: the asset's first fetch in a session waits for
/// it (capture: 0.51, 0.56, 0.65, 0.71 and 1.10 s; playtest: 0.53, 0.53 and 0.55 s), a mesh
/// fetched before resumes at the first Heartbeat after its decode, this long a vertex (capture:
/// 1,487 vertices 3.3-4.7 ms, 2,361 9-11 ms, 13,515 15-19 ms; playtest 13,515 26 ms).
pub const MESH_FETCH: f64 = 0.55;
pub const MESH_DECODE_VERTEX: f64 = 1.2e-6;

/// An animation asset's first fetch in a session (a playtest's DataModel: a new playtest fetches
/// again), from the first LoadAnimation or ContentProvider:PreloadAsync that asks for it; tracks and
/// preloads of an asset on its way share its fetch, and one fetched before loads at once. Measured
/// from the request to the Heartbeat that first sees Length: a game's playtests (four
/// scene tracks: 0.28, 0.43, 0.48, 0.70 s) and a capture Studio's playtests (31 first fetches of
/// a game's animations, 11 KB to 2.1 MB: 0.24-0.88 s, median 0.41, one 1.65; no trend with size).
pub const ANIMATION_FETCH: f64 = 0.45;

/// What the work clock is beside a client's own time where no profile weighs it by its own work (`ree profile try`'s comparison): the
/// held-out bands above, and what is not modelled. A profile's summary gives its own band (profiler/report.rs fidelity_json), so nobody
/// reads the clock as a client's where it is not one.
pub const FIDELITY: &str = "fidelity: the work clock reads 0.63-1.35x a client's time on all 182 held-out rows of the parity bench (10th to 90th percentile; Luau 0.51-1.42x including cache-tiered large buffers, engine calls 0.67-1.29x, the collector 0.70-0.96x, dispatch 0.88-1.92x), and a game's held-out per-frame code and attack casts 0.53-1.06x with their cold charges (measured before the size fit); a profile weighs these by its own work (profile.json fidelity). Not modelled: code and data a client finds cold after frames unused (a held-out cast's own Luau reads 0.3-1.0x a client), where data lies in memory (made together in one loop it reads dear), what a layout leaves warm, and the cold of each node a large walk visits (priced once a call). Size a cut in a client";

/// What the Luau VM did since it started (vendor/luau0-src, patches/luau/02-work-counters.patch: lua_workcounters)
#[repr(C)]
pub struct VmCounters {
    /// Instructions the interpreter dispatched for scripts, by opcode (Luau's Bytecode.h order)
    pub ops: [u64; 256],
    /// Bytes of the collectable objects a script's own instructions allocated
    pub allocated: u64,
    /// C functions a script called
    pub ccalls: u64,
    /// C functions called as metamethods of a script's instructions
    pub tmcalls: u64,
    /// Instructions of the engine's own Luau ([`engine_luau`]), not charged
    pub hostops: [u64; 256],
    /// Engine Luau functions a script called (a datatype's constructor, method or operator)
    pub hostcalls: u64,
    /// Collectable objects a script's own instructions allocated (tables, closures, strings, upvalues)
    pub objects: u64,
    /// Bytes of table storage they allocated (array and hash parts, a grown array's new size)
    pub stored: u64,
    /// Keys a script's own instructions made tables rehash as they grew
    pub rehashed: u64,
    /// Nanoseconds native datatype operations charged scripts (vector3.rs `charge_host_nanos`; signed)
    pub hostnanos: i64,
    /// Levels of muting the engine's Luau holds while it prices a whole call (stubs.rs `Mute`)
    pub muted: u64,
    /// `.Magnitude` reads the VM answered on a vector for a script's own instruction
    pub vectormagnitudes: u64,
    /// `.Unit` reads the same way
    pub vectorunits: u64,
    /// Method calls a script's own instruction made on a vector (NAMECALL)
    pub vectormethods: u64,
    /// GETIMPORTs of a script's instructions that looked their global up: an environment getfenv / setfenv made unsafe, or an import
    /// that was nil when the script loaded (a safe environment's imports are resolved at load, as in Roblox)
    pub slowimports: u64,
    /// Instructions of the functions a code generator would compile (a `--!native` module's but its cold ones, `@native` ones),
    /// by opcode: counted here and not in `ops`, whether or not this VM compiles them
    pub nativeops: [u64; 256],
    /// Numbers a script's work turned into strings (concatenation, tostring)
    pub numtostr: u64,
    /// Comparisons `table.sort` made for a script, and its calls of a script's comparison function
    pub sortcompares: u64,
    pub sortcalls: u64,
    /// Elements `table.create` filled, slots `table.clone` copied, items `table.concat` joined, parts `string.split` made
    pub createitems: u64,
    pub cloneitems: u64,
    pub concatitems: u64,
    pub splitparts: u64,
    /// `string.format` calls (an interpolated string is one) and the specifiers they formatted
    pub formats: u64,
    pub formatspecs: u64,
    /// Strings a script's work made that the VM did not hold already
    pub newstrings: u64,
    /// Tables, Luau closures and functions a script's work touched first in a frame (lua_coldframe), made before the frame began,
    /// and the instructions of those functions (their code's size): [`LUAU_COLD_TABLE`] and below
    pub coldtables: u64,
    pub coldclosures: u64,
    pub coldprotos: u64,
    pub coldcode: u64,
}

unsafe extern "C-unwind" {
    pub fn lua_workcounters(state: *mut mlua::ffi::lua_State) -> *const VmCounters;
    fn lua_setworkexempt(state: *mut mlua::ffi::lua_State, index: std::ffi::c_int);
    fn lua_workmute(state: *mut mlua::ffi::lua_State, on: std::ffi::c_int);
    fn lua_setworkcounting(state: *mut mlua::ffi::lua_State, on: std::ffi::c_int);
    fn lua_setsafesandbox(state: *mut mlua::ffi::lua_State, index: std::ffi::c_int);
    fn lua_setsafeglobalbase(state: *mut mlua::ffi::lua_State, index: std::ffi::c_int);
    fn lua_setgcclock(state: *mut mlua::ffi::lua_State, clock: Option<unsafe extern "C" fn(*mut std::ffi::c_void) -> f64>, ud: *mut std::ffi::c_void);
    fn lua_coldtracking(state: *mut mlua::ffi::lua_State, on: std::ffi::c_int);
    fn lua_coldframe(state: *mut mlua::ffi::lua_State);
}

/// Turns the VM's cold touches on or off (patches/luau/13-cold-touches.patch `lua_coldtracking`): on, the first touch in a frame of a table, Luau closure
/// or function made before the frame is counted (VmCounters `coldtables` and below) and priced ([`luau`]). A host that charges scripts'
/// Luau (it interprets them) turns it on; nothing a script reads changes either way.
pub fn cold_tracking(lua: &mlua::Lua, on: bool) {
    unsafe { lua_coldtracking(lua.current_thread().state(), on as std::ffi::c_int) }
}

/// A frame begins for the VM's cold touches: every object is cold again ([`cold_tracking`]).
pub fn cold_frame(lua: &mlua::Lua) {
    unsafe { lua_coldframe(lua.current_thread().state()) }
}

/// The garbage collector's clock: the scripts' own Luau work so far, measured at fixed prices ([`collector_work`]).
/// Luau paces its collector by the time a cycle took (lgc.cpp getheaptrigger: the allocation rate and the mark phase's length set
/// the next cycle's trigger), read from the wall clock, so on a busy machine the collector stepped at other points in every run:
/// `string.sub` and the other allocating fast calls give way to their C function while a step is due, a string is made again or
/// found alive, and the work clock's counts moved by a few instructions and calls; a script that slices its work by an os.clock
/// budget then took other frames (a cutscene recorded differently in 1 run of 20 to 2 of 3 cold runs, and between a one-shot run and
/// a warm service's). On this clock it steps at the same points in every run. The engine calls' charges stay out of it: the engine's
/// own bookkeeping charges some (mounting the harness enters the DataModel), and a one-shot run and a service do that differently.
struct GcClock {
    counters: *const VmCounters,
}

unsafe extern "C" fn gc_clock(ud: *mut std::ffi::c_void) -> f64 {
    // (called from inside an allocation: reads the VM's own counters, nothing that could be borrowed or raise)
    let clock = unsafe { &*(ud as *const GcClock) };
    collector_work(unsafe { &*clock.counters })
}

/// The scripts' work as the collector's clock measures it: the counters at the prices the work clock had when the collector took
/// this clock (an instruction 1.6 ns, a call 8.5 more, a fast call 2.3 more, an object 22, ...; a native module's instructions as
/// interpreted ones), not [`luau`]'s, so that a price measured anew does not move where the collector steps: its points move what
/// the counters see (an allocating fast call gives way to its C function while a step is due) and every memory reading a script
/// takes. (Priced at [`luau`]'s, the parity bench's 24 new strings read -208 bytes: a step fell inside every round.)
pub fn collector_work(c: &VmCounters) -> f64 {
    const CALLS: [usize; 2] = [20, 21];
    const FASTCALLS: [usize; 5] = [60, 68, 73, 74, 75];
    let ops: u64 = c.ops.iter().sum::<u64>() + c.nativeops.iter().sum::<u64>();
    let calls: u64 = CALLS.iter().map(|&op| c.ops[op] + c.nativeops[op]).sum();
    let fastcalls: u64 = FASTCALLS.iter().map(|&op| c.ops[op] + c.nativeops[op]).sum();
    1e-9 * (ops as f64 * 1.6
        + calls as f64 * 8.5
        + fastcalls as f64 * 2.3
        + c.objects as f64 * 22.0
        + c.stored as f64 * 0.4
        + c.rehashed as f64 * 12.0
        + c.ccalls as f64 * 5.0
        + c.hostcalls as f64 * 15.0
        + c.hostnanos as f64
        + c.vectormagnitudes as f64 * 12.5
        + c.vectorunits as f64 * 9.1
        - c.vectormethods as f64 * 9.0)
}

unsafe extern "C-unwind" {
    fn lua_sethostcfunctions(functions: *const mlua::ffi::lua_CFunction, count: std::ffi::c_int, trampoline: mlua::ffi::lua_CFunction);
}

/// Marks the engine's own C functions (`functions`, and the C function mlua's Rust callbacks run through) for the VM's library counts:
/// the objects one makes for a script are part of the operation the engine prices whole (patches/luau/15-input-size-work.patch
/// lua_sethostcfunctions), so only the VM's library functions' objects are priced as allocations.
pub fn mark_host_functions(lua: &mlua::Lua, functions: &[mlua::ffi::lua_CFunction]) -> mlua::Result<()> {
    // Every function mlua makes from a Rust closure is a C closure of one trampoline. Ask for it through a closure made here, and
    // register it with the functions in one call, before a script can have a closure classified.
    let probe = lua.create_function(|_, ()| Ok(()))?;
    unsafe {
        lua.exec_raw::<()>(&probe, |state| {
            if let Some(trampoline) = mlua::ffi::lua_tocfunction(state, -1) {
                lua_sethostcfunctions(functions.as_ptr(), functions.len() as std::ffi::c_int, trampoline);
            }
        })?;
    }
    Ok(())
}

/// The VM's collector paces itself by the scripts' work (`GcClock`); the clock is owned by the Lua state (its app data), so it lives
/// as long as the collector that reads it.
pub fn install_gc_clock(lua: &mlua::Lua, counters: *const VmCounters) -> mlua::Result<()> {
    struct Owned(#[allow(dead_code)] Box<GcClock>);
    let clock = Box::new(GcClock { counters });
    let ud = &*clock as *const GcClock as usize;
    lua.set_app_data(Owned(clock));
    unsafe { lua.exec_raw::<()>((), |state| lua_setgcclock(state, Some(gc_clock), ud as *mut std::ffi::c_void)) }
}

/// Mutes (or unmutes) the native datatype charges of the VM while the engine's Luau prices a call as a whole (`R.Mute`).
pub fn mute_host_nanos(lua: &mlua::Lua, on: bool) {
    unsafe { lua_workmute(lua.current_thread().state(), on as std::ffi::c_int) }
}

/// Turns the VM's work counters on or off (patches/luau/21-work-counting-toggle.patch).
/// Off, scripts run without per-instruction or per-allocation counting and the collector paces itself by the wall clock.
pub fn set_work_counting(lua: &mlua::Lua, on: bool) {
    unsafe {
        let _ = lua.exec_raw::<()>((), |state| {
            lua_setworkcounting(state, on as std::ffi::c_int);
            if !on {
                lua_setgcclock(state, None, std::ptr::null_mut());
            }
        });
    }
}

/// Marks a Luau chunk (compiled with mutable sandbox globals) so `setfenv` can preserve `safeenv`
/// when the target sandbox environment only overrides those globals over the immutable shared base.
pub fn set_safe_sandbox(lua: &mlua::Lua, function: &mlua::Function) -> mlua::Result<()> {
    unsafe { lua.exec_raw::<()>(function, |state| lua_setsafesandbox(state, -1)) }
}

/// Registers the immutable shared global base table for safe-sandbox `setfenv` preservation.
pub fn set_safe_global_base(lua: &mlua::Lua, table: &mlua::Table) -> mlua::Result<()> {
    unsafe {
        lua.exec_raw::<()>(table, |state| {
            mlua::ffi::lua_setreadonly(state, -1, 1);
            mlua::ffi::lua_setsafeenv(state, -1, 1);
            lua_setsafeglobalbase(state, -1);
        })
    }
}

/// Marks the engine's own Luau (the prelude, services, vector3.luau, record wrappers): Studio runs
/// that work in C++, so its instructions and what it allocates are not charged as a script's; a
/// script's call into it is ([`LUAU_HOSTCALL`]).
pub fn engine_luau(lua: &mlua::Lua, function: mlua::Function) -> mlua::Result<mlua::Function> {
    unsafe { lua.exec_raw::<()>(&function, |state| lua_setworkexempt(state, -1))? };
    Ok(function)
}

/// The seconds scripts' own Luau work so far cost: each instruction by its kind ([`INTERPRETED_OPS`], and [`NATIVE_OPS`] for a
/// `--!native` module's, which a client compiles to machine code and runs 0.8-7.8x faster), the objects and table storage it makes,
/// the C and engine functions it calls, the imports it looks up ([`LUAU_SLOW_IMPORT`]) and the library's own loops (sorting, string
/// making, formatting: [`LUAU_NUMBER_STRING`] and below), which no instruction count sees. Linear in the counts (the profiler books
/// them count by count: profiler/vm.rs `prices`).
pub fn luau(c: &VmCounters) -> f64 {
    let interpreted: f64 = c.ops.iter().zip(INTERPRETED_OPS.iter()).map(|(&count, &price)| count as f64 * price).sum();
    let native: f64 = c.nativeops.iter().zip(NATIVE_OPS.iter()).map(|(&count, &price)| count as f64 * price).sum();
    1e-9 * (interpreted
        + native
        + c.objects as f64 * LUAU_OBJECT
        + c.stored as f64 * LUAU_STORED
        + c.rehashed as f64 * LUAU_REHASHED
        + c.ccalls as f64 * LUAU_CCALL
        + c.hostcalls as f64 * LUAU_HOSTCALL
        + c.hostnanos as f64
        + c.vectormagnitudes as f64 * VECTOR_MAGNITUDE
        + c.vectorunits as f64 * VECTOR_UNIT
        + c.vectormethods as f64 * VECTOR_METHOD
        + c.slowimports as f64 * LUAU_SLOW_IMPORT
        + c.numtostr as f64 * LUAU_NUMBER_STRING
        + c.newstrings as f64 * LUAU_NEW_STRING
        + c.sortcompares as f64 * LUAU_SORT_COMPARE
        + c.sortcalls as f64 * LUAU_SORT_CALL
        + c.createitems as f64 * LUAU_CREATE_ITEM
        + c.cloneitems as f64 * LUAU_CLONE_ITEM
        + c.concatitems as f64 * LUAU_CONCAT_ITEM
        + c.splitparts as f64 * LUAU_SPLIT_PART
        + c.formats as f64 * LUAU_FORMAT
        + c.coldtables as f64 * LUAU_COLD_TABLE
        + c.coldclosures as f64 * LUAU_COLD_CLOSURE
        + c.coldprotos as f64 * LUAU_COLD_PROTO
        + c.coldcode as f64 * LUAU_COLD_CODE
        + c.hostops[COLD_CFUNCS_SLOT] as f64 * LUAU_COLD_CFUNC
        + c.hostops[COLD_HOSTFUNCS_SLOT] as f64 * LUAU_COLD_HOSTFUNC
        + c.hostops[RARE_PROTOS_SLOT] as f64 * LUAU_RARE_PROTO
        + c.hostops[SCANNED_SLOT] as f64 * LUAU_SCANNED
        + c.hostops[MOVED_SLOT] as f64 * LUAU_MOVED
        + c.hostops[STRBYTES_SLOT] as f64 * LUAU_STRING_BYTE
        + c.hostops[STRSCANNED_SLOT] as f64 * LUAU_STRING_SCANNED
        + c.hostops[MATCHSTEPS_SLOT] as f64 * LUAU_MATCH_STEP
        + c.hostops[BUFBYTES_SLOT] as f64 * LUAU_BUFFER_BYTE
        + c.hostops[ALLOCBYTES_SLOT] as f64 * LUAU_ALLOCATED_BYTE
        + c.hostops[GCOBJECTS_SLOT] as f64 * LUAU_COLLECTOR_OBJECT
        + c.hostops[LIBOBJECTS_SLOT] as f64 * LUAU_LIBRARY_OBJECT
        + c.hostops[CASEBYTES_SLOT] as f64 * LUAU_CASE_BYTE
        + c.hostops[UTF8BYTES_SLOT] as f64 * LUAU_UTF8_BYTE
        + c.hostops[UTF8CHARS_SLOT] as f64 * LUAU_UTF8_CHAR
        + c.hostops[STRCHARS_SLOT] as f64 * LUAU_STRING_CHAR
        + c.hostops[BYTEVALUES_SLOT] as f64 * LUAU_BYTE_VALUE
        + c.hostops[BUFFERS_SLOT] as f64 * LUAU_BUFFER
        + c.hostops[MATCHCLASSES_SLOT] as f64 * LUAU_MATCH_CLASS
        + c.hostops[HASHREHASHED_SLOT] as f64 * LUAU_HASH_REHASHED
        + c.hostops[LARGEBLOCKS_SLOT] as f64 * LUAU_LARGE_BLOCK
        + c.hostops[BUFFRESH_SLOT] as f64 * LUAU_BUFFER_FRESH_BYTE
        + c.hostops[BUFBIGCOPY_SLOT] as f64 * LUAU_BUFFER_BIG_COPY_BYTE
        + c.hostops[LARGEBUFFER_SLOT] as f64 * LUAU_LARGE_BUFFER
        + c.hostops[BUFDRAMCOPY_SLOT] as f64 * LUAU_BUFFER_DRAM_COPY_BYTE)
}

/// The collector's charge so far on the work clock (seconds at desktop prices): the objects the scripts' work allocated at
/// [`LUAU_COLLECTOR_OBJECT`] (a profile's `collector_ms`; it is inside [`luau`]'s total)
pub fn collector_seconds(c: &VmCounters) -> f64 {
    1e-9 * c.hostops[GCOBJECTS_SLOT] as f64 * LUAU_COLLECTOR_OBJECT
}

/// The VM's cold and first-use charges so far on the work clock (seconds at desktop prices): tables, closures and functions first
/// touched in a frame, the code functions ran first in it, and the first calls of C and engine functions ([`luau`]'s cold terms)
pub fn cold_seconds(c: &VmCounters) -> f64 {
    1e-9 * (c.coldtables as f64 * LUAU_COLD_TABLE
        + c.coldclosures as f64 * LUAU_COLD_CLOSURE
        + c.coldprotos as f64 * LUAU_COLD_PROTO
        + c.coldcode as f64 * LUAU_COLD_CODE
        + c.hostops[COLD_CFUNCS_SLOT] as f64 * LUAU_COLD_CFUNC
        + c.hostops[COLD_HOSTFUNCS_SLOT] as f64 * LUAU_COLD_HOSTFUNC
        + c.hostops[RARE_PROTOS_SLOT] as f64 * LUAU_RARE_PROTO)
}

/// The kinds of each 8-byte slot of [`VmCounters`] for a profile's bookings ([`kind`] bits; profiler/vm.rs `split`): the cold and
/// first-use counts and the collector's objects. Every other VM count (including `hostcalls`, `vectormagnitudes`, and `vectorunits`)
/// is priced from the parity bench's client sessions.
pub fn slot_kinds() -> Vec<i8> {
    use std::mem::offset_of;
    let slots = std::mem::size_of::<VmCounters>() / 8;
    let mut kinds = vec![0i8; slots];
    let slot = |offset: usize| offset / 8;
    let hostops = slot(offset_of!(VmCounters, hostops));
    for offset in [offset_of!(VmCounters, coldtables), offset_of!(VmCounters, coldclosures), offset_of!(VmCounters, coldprotos), offset_of!(VmCounters, coldcode)] {
        kinds[slot(offset)] = kind::COLD as i8;
    }
    for special in [COLD_CFUNCS_SLOT, COLD_HOSTFUNCS_SLOT, RARE_PROTOS_SLOT] {
        kinds[hostops + special] = kind::COLD as i8;
    }
    kinds[hostops + GCOBJECTS_SLOT] = kind::COLLECTOR as i8;
    kinds
}

/// The slots of [`VmCounters`]'s hostops no opcode reaches, where the VM counts a script's first call in a frame of a C function
/// and of a function of the engine's own Luau (patches/luau/13-cold-touches.patch LUA_WORK_COLDCFUNCS, LUA_WORK_COLDHOSTFUNCS): the counters keep
/// their size, since the VM state's size counts in what collectgarbage("count") reads.
pub const COLD_CFUNCS_SLOT: usize = 255;
pub const COLD_HOSTFUNCS_SLOT: usize = 254;
/// ... and the functions of a script whose code ran first in a frame without having run in the frame before (LUA_WORK_RAREPROTOS)
pub const RARE_PROTOS_SLOT: usize = 253;
/// ... and what the library and engine calls a script makes do by the size of their input (patches/luau/15-input-size-work.patch,
/// lua.h LUA_WORK_SCANNED and below): table elements table.find compared; elements table.insert / table.remove at a position moved,
/// table.move copied, table.unpack / table.pack / string.byte pushed and table.clear cleared; bytes of the strings the script's work
/// made or looked up (built, hashed, compared: concatenation, the string library, the engine's strings); bytes a plain search or a
/// utf8 function read; the pattern matcher's steps; bytes a buffer function zeroed, filled, copied or wrote; the bytes of every object
/// and table part the script's work allocated; every object it allocated again, for the collector; objects library calls made for it
pub const SCANNED_SLOT: usize = 252;
pub const MOVED_SLOT: usize = 251;
pub const STRBYTES_SLOT: usize = 250;
pub const STRSCANNED_SLOT: usize = 249;
pub const MATCHSTEPS_SLOT: usize = 248;
pub const BUFBYTES_SLOT: usize = 247;
pub const ALLOCBYTES_SLOT: usize = 246;
pub const GCOBJECTS_SLOT: usize = 245;
pub const LIBOBJECTS_SLOT: usize = 244;
/// ... bytes string.upper / lower converted, bytes a utf8 function decoded or stepped over, codepoints utf8.char encoded, characters
/// string.char made, values string.byte pushed, buffers made, and the pattern matcher's steps that tested a character class
pub const CASEBYTES_SLOT: usize = 243;
pub const UTF8BYTES_SLOT: usize = 242;
pub const UTF8CHARS_SLOT: usize = 241;
pub const STRCHARS_SLOT: usize = 240;
pub const BYTEVALUES_SLOT: usize = 239;
pub const BUFFERS_SLOT: usize = 238;
pub const MATCHCLASSES_SLOT: usize = 237;
/// ... of the keys a script's table growth rehashed (`rehashed`), those it put in a hash part; and the allocations of a script's work
/// too large for Luau's own pages (over 1,024 bytes), each a block of the host's allocator
pub const HASHREHASHED_SLOT: usize = 236;
pub const LARGEBLOCKS_SLOT: usize = 235;
/// ... and of the bytes buffer functions handled, a new buffer's past Luau's own pages (over 1 KB), each large buffer (>= 64 KB),
/// and a copy's, a fill's or a written string's in a call over 64 KB (L2/L3 tier) and over 1 MB (DRAM tier)
/// (lua.h LUA_WORK_BUFFRESH, LUA_WORK_BUFBIGCOPY, LUA_WORK_LARGEBUFFER, LUA_WORK_BUFDRAMCOPY)
pub const BUFFRESH_SLOT: usize = 234;
pub const BUFBIGCOPY_SLOT: usize = 233;
pub const LARGEBUFFER_SLOT: usize = 232;
pub const BUFDRAMCOPY_SLOT: usize = 231;
/// The first of hostops' slots that count something other than an opcode (lua.h LUA_WORK_FIRSTSLOT)
pub const FIRST_SPECIAL_SLOT: usize = 231;

/// The engine instructions hostops counted: its opcodes' slots, without the slots that count other work
pub fn host_instructions(c: &VmCounters) -> u64 {
    c.hostops[..FIRST_SPECIAL_SLOT].iter().sum::<u64>()
}



/// Cold per-frame Luau, in nanoseconds: what a client pays the first time in a frame a script touches a table, calls a Luau closure or
/// runs a function made before the frame (VmCounters `coldtables` and below; patches/luau/13-cold-touches.patch `lua_coldframe`), beyond the hot
/// prices above. A client runs its engine work (layout, rendering, physics) between two frames' scripts, so code that runs once a
/// frame over data it made over time finds both out of the CPU's caches, which a hot loop never does. Measured in rendered clients of
/// a game's lobby and match place (roblox-sim, Studio 0.744; notes/macro coldprobe, six sessions): records walked once a frame cost
/// over the same walk hot 14 ns a record made together in one loop, 50-70 spread among other allocations, 85-180 made one a frame,
/// 140-210 made over many frames between instances and other allocations (a walk of the same records as a list costs the same:
/// it is where they lie, not the walk), more for wide records (16 keys: 110-125); a closure 41-59 ns (made spread), a module's
/// function about 16 with its closure (made together at load), and a long function 0.85 ns an instruction of its code (600
/// statements once a frame: 5.5 ns a statement against 2.5). A game's per-frame data is made over time: the table price is the
/// spread end, which the game's own per-frame code bears out (notes/macro: a gradient step's 900 records 207-238 ns each over the
/// hot prices, the per-frame code of a scrolled grid, of an effect system and of a server's enemy loop at 0.76-1.07x a client with
/// it). Data made together in one loop reads up to 14x dear: the VM does not see where an object lies.
pub const LUAU_COLD_TABLE: f64 = 200.0;
/// A script's first call in a frame of a C function (the libraries, the task and coroutine functions) and of an operation of a
/// datatype the engine serves (a constructor, a method, an operator), made before the frame, in nanoseconds: the engine's code for it
/// runs cold, as an Instance member's does ([`FIRST_CALL`]). Measured in rendered clients of a game's match place (roblox-sim,
/// Studio 0.744; notes/macro pathprobe, two sessions, and two headless ones alike: each case in a frame of its own after a few calls
/// every frame makes anyway, the frame's first call less a second right after, less the 0.7 us its own function's first run adds,
/// [`LUAU_RARE_PROTO`]): task.spawn of a function 4.0 us, of a parked coroutine 2.0, coroutine.resume 1.9, task.defer 3.0, task.delay
/// 3.5, string.format 3.3, table.clone 1.1, math.noise 0.9 (the median 2.5); CFrame.new 0.55, CFrame.fromOrientation 0.85, a CFrame
/// product 1.4, CFrame:Lerp 2.5, Color3.new 0.9, NumberSequence.new 1.5, UDim2.fromOffset 0.8 (the median 0.9). A builtin a script's
/// fast call runs (table.insert, math.floor, typeof) is the VM's own code here and pays none (a client's table.insert 1.2, typeof 0.6:
/// not modelled).
pub const LUAU_COLD_CFUNC: f64 = 2500.0;
pub const LUAU_COLD_HOSTFUNC: f64 = 900.0;
/// What a function's first run in a frame adds when its code did not run in the frame before, in nanoseconds: per-frame code finds its
/// code in a cache the frame before left (the prices above, as the coldprobe measured: a module's function about 16 ns with its
/// closure), and code a cast runs now and then, an effect's setup, finds it nowhere. The pathprobe's call of a function of 30
/// instructions whose code last ran about 20 frames before, from a case function that last ran then too: 1.75 us its first call in a
/// frame, 0.35 the next closure of it: 0.7 us each of the two functions.
pub const LUAU_RARE_PROTO: f64 = 700.0;
pub const LUAU_COLD_CLOSURE: f64 = 30.0;
pub const LUAU_COLD_PROTO: f64 = 5.0;
pub const LUAU_COLD_CODE: f64 = 0.85;

/// The kinds of instruction priced apart (tools/cost-parity/fit.py CLASSES), by opcode (Luau's Bytecode.h order): what a code generator
/// turns into a few machine instructions (arithmetic, moves, jumps, comparisons), `%` / `//` / `^`, keyed table reads and writes,
/// dynamic ones, allocations (a table, closure, concatenation, varargs), calls and returns, a generalized loop's step, a builtin's
/// fast call, globals and imports, and a method call's lookup. The opcodes Luau 0.746 added take their kind's class: CALLFB is the
/// call a client's compiler emits (LuauEmitCallFeedback), the userdata ones a host's direct member access, the class ones
/// user-defined classes (a debug flag). NAMECALL is priced apart: 0.746's interpreter dispatches the CALL after it as an instruction
/// of its own, where 0.694 fell through to it and counted the two as one NAMECALL, the count the call class's interpreted price was
/// fitted on; so that price is the CALL's, and the lookup's is the rest of what the fit gave a method call (nothing). Native code
/// always ran the two apart. FASTPCALL, which 0.747's compiler emits for every pcall and xpcall, is a call too, and the VM counts the
/// C call of pcall it stands for: a pcall costs its call and that C call (only the import the fallback carries is skipped), about what
/// a 0.747 client's costs (the parity bench's lang.call.pcall 18.9 ns against the client's 18.3).
const fn op_class(op: usize) -> usize {
    match op {
        // MOD, POW, MODK, POWK, IDIV, IDIVK
        37 | 38 | 43 | 44 | 81 | 82 => 1,
        // GETTABLEKS, SETTABLEKS, GETTABLEN, SETTABLEN, LENGTH, SETLIST, GETUDATAKS, SETUDATAKS, GETPRIVATEFIELD, SETPRIVATEFIELD
        15..=18 | 52 | 55 | 83 | 84 | 93 | 94 => 2,
        // GETTABLE, SETTABLE
        13 | 14 => 3,
        // NEWCLOSURE, CONCAT, NEWTABLE, DUPTABLE, GETVARARGS, DUPCLOSURE, NEWCLASS, CONSTRUCT
        19 | 49 | 53 | 54 | 63 | 64 | 90 | 91 => 4,
        // CALL, RETURN, NATIVECALL, NAMECALLUDATA, CALLFB, FASTPCALL
        21 | 22 | 62 | 85 | 87 | 89 => 5,
        // FORGLOOP, FORGPREP_INEXT, FORGPREP_NEXT, FORGPREP
        58 | 59 | 61 | 76 => 6,
        // FASTCALL3, FASTCALL, FASTCALL1, FASTCALL2, FASTCALL2K
        60 | 68 | 73 | 74 | 75 => 7,
        // GETGLOBAL, SETGLOBAL, GETIMPORT
        7 | 8 | 12 => 8,
        // NAMECALL
        20 => 9,
        _ => 0,
    }
}

/// An opcode's price from its class's ([`op_class`]'s order)
const fn priced(by_class: [f64; 10]) -> [f64; 256] {
    let mut prices = [0.0; 256];
    let mut op = 0;
    while op < 256 {
        prices[op] = by_class[op_class(op)];
        op += 1;
    }
    prices
}

/// Nanoseconds an interpreted instruction costs, by opcode: a fit on the parity bench's interpreted language rows (31 workloads and
/// the same loops in a module getfenv deoptimized; non-negative least squares on relative error, fit.py luau --free slowimport; a
/// rendered client of a game's match place, three sessions of the whole corpus) of a price for each kind of instruction, the other
/// counts at their prices: the 21 fit rows within 7.8 % at the median (the worst a trigonometry loop, 0.62x: math.sin's own work is a
/// fast call's price here), the 14 held-out rows 22.9 % (the worst a loop that calls a local function, 0.29x). Arithmetic, moves,
/// jumps and comparisons 0.48 ns; `%` / `//` / `^` 5.5; a keyed table read or write 2.5, a dynamic one 2.2; a call or return 4.9 (a
/// method call's NAMECALL nothing more: its CALL is the call); a generalized loop's step 2.5; a builtin's fast call 3.8; a global or a resolved import 1.4; an allocation nothing
/// beyond its object ([`LUAU_OBJECT`]). The prices fitted before, on a Studio playtest client (tools/luau-cost: 1.6 an instruction, 8.5
/// a call more, 2.3 a fast call more), read the bench's interpreted rows 0.45-2.1x (a math loop 2.0x, a recursive call 1.9x).
pub const INTERPRETED_OPS: [f64; 256] = priced([0.48, 5.46, 2.45, 2.20, 0.0, 4.89, 2.54, 3.75, 1.36, 0.0]);

/// Nanoseconds an instruction of a native module costs, by opcode: the same fit on the bench's native rows (fit.py luau --native; the
/// 20 fit rows within 30.5 % at the median, the worst a dynamic key's loop 0.27x; the 9 held-out rows 39 %, the worst a closure's
/// loop 1.7x). A code generator turns arithmetic, moves, jumps and keyed table reads into a few machine instructions (0.42 ns), a dynamic
/// key's lookup and `%` / `//` / `^` stay calls into the VM (0.65-0.75), an allocation (a table, closure, concatenation) 1.7 more than
/// its object, a call or return 3.3 and a generalized loop's step 1.9; imports are resolved at load (0.27) and a builtin's fast call is
/// inlined (0.19).
pub const NATIVE_OPS: [f64; 256] = priced([0.43, 0.65, 0.42, 0.75, 1.74, 3.25, 1.93, 0.90, 0.27, 3.25]);

/// A GETIMPORT that looks its global up (vm counter `slowimports`): a safe environment's imports are resolved when the script loads,
/// as Roblox resolves them, and read as a constant ([`INTERPRETED_OPS`]' global class); one getfenv or setfenv made unsafe looks every
/// import up again, and so does an import that was nil at load (the parity bench: two imports a loop step 4.6 ns in a module, 21.4 in
/// one that called getfenv first; the fit above gives a looked-up import 7.9 more than a resolved one).
pub const LUAU_SLOW_IMPORT: f64 = 7.9;

/// The library's own work, in nanoseconds, beyond its C call and the objects it makes: a fit on the parity bench's 25 fit rows that do
/// such work (the same sessions; fit.py library: within 4.1 % at the median; the 10 held-out rows 17.3 %, the worst table.clone of a
/// hash 0.59x). A number made a string (tostring(n), `..` on a number) 18; a string no live string equals (Luau interns strings) 171
/// (`tostring` of a new number 219 ns in all, `"n" .. i` 412: a client that has run a game's worth of work pays that, and one that has
/// run only these rows a third of it, 83 and 140); table.sort's comparison 1.1 (100 numbers 1.25 us) and its call of a comparison
/// function 9.7 (50 elements 4.7 us); table.create's element 4.3 (100: 0.43 us), table.clone's slot 4.3 (an array of 10: 56 ns),
/// table.concat's item 7.1 (20: 154 ns); string.split's part 25.4 (five: 140 ns) and string.format's call 102 (an interpolated string is
/// one: 280 ns with two values, a float's %.2f 335; the specifiers' kinds differ too much for a price by count, and the fit gives them none).
/// table.create's element and table.concat's item are fitted again with the size prices below, at several sizes (1.95 and 5.0).
/// string.split's part is fitted again on a 0.747 client, whose string.split is Luau 747's faster one (DFFlagLuauOptimizeStringSplit): 11.7,
/// every other price as here, on the bench's split rows at 5, 100 and 10,000 parts (three rendered sessions of the whole corpus in the
/// match place, reference/client-747-full-*; 75 ns, 1.85 us and 142 us a call), where 25.4 read them 1.5-2.2x a client's; the held-out
/// rows at 10 and 1,000 parts read 1.04x and 1.13x (1.92x and 2.29x before).
pub const LUAU_NUMBER_STRING: f64 = 18.0;
pub const LUAU_NEW_STRING: f64 = 150.0;
pub const LUAU_SORT_COMPARE: f64 = 0.90;
pub const LUAU_SORT_CALL: f64 = 9.7;
pub const LUAU_CREATE_ITEM: f64 = 2.30;
pub const LUAU_CLONE_ITEM: f64 = 2.4;
pub const LUAU_CONCAT_ITEM: f64 = 5.0;
pub const LUAU_SPLIT_PART: f64 = 11.7;
pub const LUAU_FORMAT: f64 = 102.0;

/// What library and engine calls do by the size of their input, in nanoseconds (the VM's counts: [`SCANNED_SLOT`] and below), fitted
/// on the parity bench's `libsize`, `json` and `gc` rows, each workload at three to five sizes (tools/cost-parity/fit.py sizes; numbers
/// in its comments). A flat price per call read table.find of 100,000 elements and of 10 alike (6 us), `s ..= piece` building 16,000
/// pieces at 1.0x n where a client's time grows as n^2, and gsub, upper and rep on 1 MB at 6 us where a client takes 0.3-7 ms.
/// A table element table.find compared, and one table.insert / table.remove at a position moved, table.move copied, table.unpack /
/// pack / string.byte pushed or table.clear cleared
pub const LUAU_SCANNED: f64 = 0.357;
pub const LUAU_MOVED: f64 = 0.21;
/// A byte of a string a script's work made or looked up (built, hashed and compared; copied when new), a byte a plain search or a utf8
/// function read without making a string, a step of the pattern matcher (an item tried at a position), a byte a buffer function
/// zeroed, filled, copied or wrote
pub const LUAU_STRING_BYTE: f64 = 0.169;
pub const LUAU_STRING_SCANNED: f64 = 0.024;
pub const LUAU_MATCH_STEP: f64 = 0.70;
pub const LUAU_BUFFER_BYTE: f64 = 0.004;
/// A byte of an object or table part a script's work allocated (its instructions' and its library and engine calls'), beyond the
/// object's price ([`LUAU_OBJECT`], [`LUAU_LIBRARY_OBJECT`]): what touching fresh memory costs
pub const LUAU_ALLOCATED_BYTE: f64 = 0.061;
/// The collector's work for an object a script's work allocated, its instructions' or its library and engine calls' (lua.h
/// LUA_WORK_GCOBJECTS), booked to the code that allocated it: sweeping it once it is garbage, and marking it while it lives. On the
/// parity bench's churn rows on a heap of about 50 MB (`gc.churn.*.big_heap`) ree's VM, which runs the client's collector, spends 17.5 %
/// of a loop of four-slot tables in collector steps, and that share of a client's 49.4 ns is 8.6 ns a table. It is a price an object, not
/// a byte: a collector frees a large block at once (a client builds a 1 MB string.format, 3.7 MB of buffers, in 271 us in all, where
/// 0.12 ns a byte read 440 us for the collector alone), and the big-heap rows of 1 KB strings and 128-slot tables read what their small
/// heap twins read (815 and 832 ns, 551 and 668), so marking a game-sized heap adds little a byte allocated. The collector's own pace is
/// not this clock's (it stays [`collector_work`]'s).
pub const LUAU_COLLECTOR_OBJECT: f64 = 8.6;
/// An object a library or engine call made for a script (a string, a table JSONDecode built, a buffer), beyond its bytes and its
/// collector's share
pub const LUAU_LIBRARY_OBJECT: f64 = 5.99;
/// A byte string.upper or string.lower converted (a C library call a character), a byte a utf8 function decoded, a codepoint
/// utf8.char encoded, a character string.char made of an argument, a value string.byte pushed, a buffer made (beyond its bytes), a
/// byte a library function wrote into a string it builds piece by piece or copied as that buffer grew, and a pattern step that tested
/// a character class
pub const LUAU_CASE_BYTE: f64 = 1.485;
pub const LUAU_UTF8_BYTE: f64 = 0.979;
pub const LUAU_UTF8_CHAR: f64 = 6.71;
pub const LUAU_STRING_CHAR: f64 = 2.16;
pub const LUAU_BYTE_VALUE: f64 = 1.46;
pub const LUAU_BUFFER: f64 = 1774.0;
pub const LUAU_MATCH_CLASS: f64 = 6.33;
/// A key a table's growth reinserted into its hash part, beyond [`LUAU_REHASHED`] (an array slot is copied; a key is hashed again)
pub const LUAU_HASH_REHASHED: f64 = 15.43;
/// An allocation too large for Luau's own pages (over 1,024 bytes: a string or buffer of 1 KB, a table part of 64 slots or more),
/// beyond its bytes: a block of the host's allocator, made and freed apart (fitted with the rest)
pub const LUAU_LARGE_BLOCK: f64 = 79.8;
/// Cache-tiered pricing for large buffer operations (patches/luau/25-buffer-cache-tiers.patch), fitted on the parity bench's
/// non-held-out buffer rows (64 KB to 4 MB against the fastest kept client sessions):
/// - A new buffer past Luau's small pages (> 1 KB) pays `LUAU_BUFFER_FRESH_BYTE` on top of `LUAU_ALLOCATED_BYTE` + `LUAU_BUFFER_BYTE`
///   (~0.091 ns/byte total for fresh memory zeroing), plus `LUAU_LARGE_BUFFER` once when >= 64 KB (allocator/OS large-segment setup).
/// - A buffer.copy, buffer.fill or buffer.writestring call pays `LUAU_BUFFER_BYTE` (0.004 ns/byte) within L1/L2 (<= 64 KB), plus
///   `LUAU_BUFFER_BIG_COPY_BYTE` (0.002 ns/byte -> 0.006 total) in L2/L3 (64 KB..1 MB), plus `LUAU_BUFFER_DRAM_COPY_BYTE`
///   (0.030 ns/byte -> 0.036 total) above 1 MB (DRAM bandwidth).
pub const LUAU_BUFFER_FRESH_BYTE: f64 = 0.026;
pub const LUAU_BUFFER_BIG_COPY_BYTE: f64 = 0.002;
pub const LUAU_LARGE_BUFFER: f64 = 18_000.0;
pub const LUAU_BUFFER_DRAM_COPY_BYTE: f64 = 0.030;

/// An object a script's instruction makes, beyond its bytes and its collector's share ([`LUAU_ALLOCATED_BYTE`],
/// [`LUAU_COLLECTOR_OBJECT`]: 14.6 ns an object in all), and a byte of table storage beyond its allocated byte (array and hash parts):
/// fitted with the size prices (above), where the Studio playtest's fit had 22 and 0.4
pub const LUAU_OBJECT: f64 = 5.99;
pub const LUAU_STORED: f64 = 0.079;
/// A key moved when a table grows, beyond [`LUAU_HASH_REHASHED`] for one put in a hash part: nothing (an array slot's copy is in its
/// bytes; the Studio playtest's fit had 12, and the bench's held-out list growth read 2.8x a client); a C function call (the Instance
/// API's own costs come on top: READ, CALL, ...): fitted with the size prices, the library's calls with them
pub const LUAU_REHASHED: f64 = 0.0;
pub const LUAU_CCALL: f64 = 5.77;
/// A datatype's operation the engine's Luau serves, client-calibrated on the parity bench's non-held-out datatype rows.
pub const LUAU_HOSTCALL: f64 = 12.0;
/// `.Magnitude` and `.Unit` on a Vector3, which the VM answers itself (patches/luau/05-vector-members.patch, luaV_vectormember),
/// client-calibrated on the parity bench's non-held-out vector rows.
pub const VECTOR_MAGNITUDE: f64 = 9.0;
pub const VECTOR_UNIT: f64 = 10.0;
/// A method call on a Vector3 (`a:Dot(b)`) beyond what its instructions and C call count: nothing. Its NAMECALL and CALL at
/// [`INTERPRETED_OPS`]' call price and the C call read `Dot` at about 11 ns, as Studio's client (`Dot`, `Cross` and `Max` 9.4-12.2 ns
/// whole) and the parity bench (Dot 11.7, Cross 12.0: held-out rows) do; the older call price (8.5 ns more a call) needed -9 here.
pub const VECTOR_METHOD: f64 = 0.0;
/// `Vector3:Lerp` and `:FuzzyEq` are still Luau functions of the engine (a host call each, src/lua/vector3.luau), charged on top of what
/// they count, in seconds as the other `R.Charge` figures: Lerp nothing (the parity bench reads it at 14.9 ns whole, below what its
/// calls count here: 22), FuzzyEq what Studio's rig frame read over the counts then (the bridge_ops probe, a match place's client,
/// three runs, six readings each: 35.9 ns against 19.3; no bench row)
pub const VECTOR_LERP: f64 = 0.0;
pub const VECTOR_FUZZY_EQ: f64 = 16.5e-9;
/// An InstanceHandle's `:Get()` of an instance and `==` (lua/types.rs LInstanceHandle) beyond what their calls count, fitted on
/// the parity bench's fit rows (a rendered 0.747 client of a game's match place, six sessions; each row's fastest, as these rows
/// read up to twice as dear in some sessions, as allocating rows do with the collector's phase): `Get` of a parented part 30.2
/// ns whole, where the namecall's call counts 15.3 here, and of none 12.2 (24.2-24.7 in the other sessions, never more than
/// half the part's in a session: none is not charged); `==` of two handles of one instance 25.2 and of two instances 26.3,
/// where the metamethod counts 1.6. Held out: `Get` of a loose folder 23.9 (ree 26.3). InstanceHandle.new is the served
/// call's price as it is: 45.7 ns of an instance (ree 55.6) and, held out, 27.6 of none (ree 22.3).
pub const HANDLE_GET: f64 = 15.0e-9;
pub const HANDLE_EQ: f64 = 24.0e-9;
