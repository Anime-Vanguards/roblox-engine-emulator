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
/// the miss belongs to the instance, not the property). The hot price of a read is 35-45 ns, so the first touch adds about this.
pub const COLD_TOUCH: f64 = 0.28e-6;

/// What a changed write adds to the first touch of an instance in a frame beyond [`COLD_TOUCH`]. A write dirties more of the
/// instance than a read touches (the property's slot, its change flags, the render or physics structure that hears of it), so a roster
/// of 100 distinct instances written once each costs more than 100 times the loop's price plus a read's miss: a cold_write probe in
/// a match place's client, the medians of its runs, per write fitted from the totals at 100 and 300 instances, less the same writes on
/// one instance: Transparency +388 ns, Color +357, CFrame +773, Name +559, SetAttribute +830,
/// against ree's 280 (a read's miss). This table is the rest, so the roster of a HUD or a rig lands within 1.3x of Studio's per-write
/// slope. NOT modeled: the fixed cost a frame's first writes pay (a first_write probe: the first changed write of a frame
/// 14.1 us, a property written for the first time in it 4-18 us more, then 1-2 us for the next instance and 0.3 once a loop runs; it
/// is cold code after the render and physics steps). It belongs to the frame, not to the script that happens to write first: charged to
/// that script it would land on whichever module runs first and inflate any cut that removes it by up to 10 us a frame (a measured pair
/// of such writes is 17 us), so ree leaves it out and a roster of 100 reads that much low in total, 10-20 % for a Transparency roster.
///
/// A GuiObject (`part` false; the same bench's `Frame` rows, five runs: Size +475..525, Position +600, BackgroundTransparency +380 over
/// the hot write, noisier than the Parts': Position's n100 readings span 781-1550 ns) is dearer on Size and Position and close on the
/// transparencies.
pub fn cold_write_extra(part: bool, property: &str) -> f64 {
    1e-9 * match (part, property) {
        (true, "CFrame" | "Position" | "Orientation" | "Rotation" | "PivotOffset" | "Size") => 500.0,
        (true, "Color" | "BrickColor" | "Material") => 80.0,
        (true, "Transparency" | "LocalTransparencyModifier") => 110.0,
        (false, "Size" | "Position") => 300.0,
        (false, "BackgroundTransparency" | "TextTransparency" | "ImageTransparency") => 100.0,
        (_, "Name") => 280.0,
        (_, "SetAttribute") => 550.0,
        // (a joint's or bone's Transform and the properties not measured: the first touch alone, as fitted on Motor6D.Transform)
        _ => 0.0,
    }
}

/// The frame each instance slot was last touched in (one per engine: the scheduler owns it, the Instance API's C metamethods
/// share it like the work cell).
pub struct Touched {
    frame: u32,
    stamps: Vec<u32>,
    /// Whether the most recent [`Touched::touch`] found its instance cold
    last_cold: bool,
}

impl Touched {
    pub fn new() -> Self {
        Touched { frame: 1, stamps: Vec::new(), last_cold: false }
    }

    /// Whether the most recent touch found its instance cold (a SetAttribute's namecall touches before its body runs); asking clears it,
    /// so a later call that did no touch of its own (an engine-internal SetAttribute) never inherits it.
    pub fn take_last_cold(&mut self) -> bool {
        std::mem::take(&mut self.last_cold)
    }

    /// Frame `frame` begins (every instance is cold again).
    pub fn begin_frame(&mut self, frame: u64) {
        self.frame = frame as u32 + 1;
    }

    /// [`COLD_TOUCH`] if the instance in slot `index` has not been touched in this frame, else 0; marks it touched.
    #[inline]
    pub fn touch(&mut self, index: u32) -> f64 {
        let slot = index as usize;
        if self.stamps.len() <= slot {
            self.stamps.resize(slot + 1 + slot / 2, 0);
        }
        if self.stamps[slot] == self.frame {
            self.last_cold = false;
            0.0
        } else {
            self.stamps[slot] = self.frame;
            self.last_cold = true;
            COLD_TOUCH
        }
    }
}

// Second pass (2026-10-23): the instance operations are priced from an op_cost probe, one body run in
// Studio and in ree, the median of seven rounds less an empty loop, in an empty test place (edit) and in
// a lobby playtest's client (a game's real runtime, the figures used
// where they differ; a property write in a running place costs about 60 % of an edit write). Every figure
// below that says "client" is that bench's number.

/// A property or member read (client: Part.CFrame / Position / Size / Transparency 26-40 ns, Name 40, Parent 41,
/// GuiObject.Size 31-69; the VM's own GETTABLEKS comes on top)
pub const READ: f64 = 0.033e-6;
/// A property write nothing in the engine reacts to, and a write of the value it holds (client: CFrame written
/// to what it holds 52 ns, Bone.Transform and Motor6D.Transform 55-68 ns)
pub const WRITE_QUIET: f64 = 0.055e-6;
/// A method call the table below does not name (client: IsA 35, IsDescendantOf 46, FindFirstChild miss 56,
/// GetAttribute 62-67, FindFirstChildOfClass 61, HasTag 75, FindFirstChild hit of 20 children 93 ns, with the
/// namecall's VM cost)
pub const CALL: f64 = 0.045e-6;
/// An instance destroyed (playtest: a grass patch in Workspace 4.9 us an instance, with leaving
/// the DataModel, [`moved`]; client: Destroy of a Part out of the tree 0.6-0.9 us)
pub const DESTROYED: f64 = 1.0e-6;
/// One element of a list a method returns (client: GetChildren of 20 0.64 us, GetDescendants of 100 2.5 us;
/// EditableMesh GetFaces and GetColors 0.01-0.02 us an id)
pub const LISTED: f64 = 0.02e-6;
/// QueryDescendants: a node it looks at and a node it returns (client: ">>Part" over 100 descendants 3.6 us,
/// ">>Model > Part#Part3" 1.0 us for 10 matches, edit 2.3 and 1.2)
pub const QUERIED: f64 = 0.010e-6;
pub const QUERY_FOUND: f64 = 0.015e-6;
/// An attribute set to another value (client: SetAttribute number 0.77 us, string 0.79, a new key and its
/// removal 0.83 per call; to the value it holds 0.09, which [`CALL`] covers)
pub const ATTRIBUTE_CHANGED: f64 = 0.70e-6;
/// Signal:Connect and Connection:Disconnect (client: Connect + Disconnect 0.78 us, Part.Changed 0.58-0.84;
/// split 60/40), GetPropertyChangedSignal beyond them (1.2-1.3 us in all)
pub const CONNECT: f64 = 0.45e-6;
pub const DISCONNECT: f64 = 0.30e-6;
pub const PROPERTY_SIGNAL: f64 = 0.45e-6;
/// BindableEvent:Fire with nothing connected, beyond the call (client 161 ns, edit 183)
pub const FIRE: f64 = 0.10e-6;
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
    /// other events: fired as before, unpriced
    Unmeasured,
}

/// What a second handler and each one after it on the same signal adds (a second handler_cost probe, a match place's client, three
/// runs, a Changed signal with 10 handlers on each of 100 parts and with 100 handlers on each of 10 parts): the trigger's queueing grew by
/// 0.45 us a further handler (10 handlers 4.55-4.99 us a fire, 100 handlers 43.0-45.1; one handler 0.51-0.80, so the fire's own part is
/// the 0.20 [`SIGNAL_FIRED`]) and each handler's thread cost 0.87-1.04 us (median 0.95) where a lone handler's cost 1.6-2.8, so the first
/// handler pays the dispatch of the fire and the others only their thread. A lone handler is still priced as the earlier probes had
/// it: their hot loop of two writes read the queueing 0.23-0.35 us, this one and the first handler_cost probe's 100 instances a frame 0.5-0.8.
pub const EXTRA_HANDLER_QUEUED: f64 = 0.45e-6;
pub const EXTRA_HANDLER_THREAD: f64 = 0.95e-6;
/// One more handler of a frame event (PreSimulation, Heartbeat, PreRender, ...): 100 handlers on one event, the span from the first
/// handler's first line to the last, over 99 (a match place's client, three runs: PreSimulation 0.73, Heartbeat 0.63, PreRender 0.71 us).
pub const FRAME_HANDLER: f64 = 0.69e-6;

/// What a fire of `kind` with a handler connected costs the firer, and what each handler's thread costs when it runs (seconds). Queueing,
/// per fire (the trigger less the trigger with nothing connected, medians of three runs): Changed and the property signal keep
/// [`SIGNAL_FIRED`]'s 0.20 us (its loop of two writes read +0.23..0.35 and +0.08..0.21; this probe's 100 a frame 0.53 and 0.60, noisier), AttributeChanged
/// 0.63 / 0.51 / 0.66 and GetAttributeChangedSignal 0.82 / 0.55 / 0.55 (0.55 us), ChildAdded 0.40 / 0.38 / 0.66 (0.40), DescendantAdded
/// 0.27 / 0.25 / 0.41 (0.27). Handler thread, per fire (last handler's first line less the trigger's end, less 0.02 us for the handler's
/// own instructions): Changed 1.97 / 1.89 / 2.24 (1.83 as the median of the property signal's 1.87 too), AttributeChanged 1.71 / 1.62 /
/// 2.06 and GetAttributeChangedSignal 1.86 / 1.72 / 1.93 (1.78), ChildAdded 1.53 / 1.46 / 1.89 (1.50), DescendantAdded 1.46 / 1.45 /
/// 1.69 (1.44).
pub fn signal_costs(kind: SignalKind) -> (f64, f64) {
    match kind {
        SignalKind::Changed | SignalKind::Property => (SIGNAL_FIRED, 1.83e-6),
        SignalKind::Attribute => (0.55e-6, 1.78e-6),
        SignalKind::ChildAdded => (0.40e-6, 1.50e-6),
        SignalKind::DescendantAdded => (0.27e-6, 1.44e-6),
        // (three runs of the second handler_cost probe in a match place's client: queueing 0.42 / 0.35 / 0.65 / 0.58 us, handler thread
        // 1.47 / 1.58 / 2.17 / 1.73)
        SignalKind::ChildRemoved => (0.42e-6, 1.50e-6),
        SignalKind::DescendantRemoving => (0.35e-6, 1.58e-6),
        SignalKind::AncestryChanged => (0.65e-6, 2.17e-6),
        SignalKind::Bindable => (0.58e-6, 1.73e-6),
        SignalKind::Unmeasured => (0.0, 0.0),
    }
}
/// A thread made for task.spawn / defer / delay (client: task.spawn(noop) 1.15 us, coroutine.wrap(noop)() 1.03)
pub const THREAD: f64 = 1.0e-6;
/// BulkMoveTo, a part in the call (client, 100 parts: FireCFrameChanged 0.42 us a part, FireAllEvents 1.03; a
/// loop of CFrame writes 1.10)
pub const BULK_MOVED: f64 = 0.42e-6;
pub const BULK_MOVED_ALL_EVENTS: f64 = 1.03e-6;
/// Model:PivotTo, a part it moves (client: 0.48 us in the workspace; 0.15 a call), a Part:PivotTo 1.14 us
pub const PIVOT_PART: f64 = 0.48e-6;
pub const PIVOT_BASE: f64 = 0.15e-6;
/// ... out of the workspace, a part (a CFrame write there 0.16 us, and the walk)
pub const PIVOT_LOOSE: f64 = 0.20e-6;
pub const PIVOT_SINGLE: f64 = 1.14e-6;
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
/// Model:GetPivot / GetScale / GetBoundingBox (client: 31-60 ns, cached by the engine)
pub const MODEL_QUERY: f64 = 0.015e-6;
/// A Parent change that neither enters nor leaves the DataModel (client: a Part into another Folder and back
/// 0.94 us a move, edit 1.24); one between two places outside it costs little (not measured)
pub const REPARENTED: f64 = 1.0e-6;
pub const REPARENTED_LOOSE: f64 = 0.15e-6;
/// The parallel bridge's machinery (an empty test place's client, an actor_cost probe, one session, 2026-10-23): BindableFunction:Invoke into an
/// Actor's OnInvoke 1,025 ns (ree's call itself counts 81.5), SharedTable.increment 370-429 (counts 31.5); reads (29 ns) are close to a table's
/// and writes (298 ns) are not priced: a SharedTable is a plain table here. Charged on top of what the instructions count.
pub const ACTOR_INVOKE: f64 = 0.94e-6;
pub const SHARED_INCREMENT: f64 = 0.37e-6;
/// TextService:GetTextSize (client and edit: "Hello" 14 FredokaOne 6.7-6.9 us)
pub const TEXT_MEASURED: f64 = 6.7e-6;

/// A Text write on a TextLabel, TextButton or TextBox (a lobby client: 0.72-0.80 us with AutoLocalize off or digits only, 0.07
/// when the text is the one it has). With AutoLocalize on and letters in the text it also looks the text up for translation in a
/// place that has a localization table: [`text_localization_in`].
pub const TEXT_WRITE: f64 = 0.75e-6;

/// The places whose localization table Studio searches on a Text write (AutoLocalize on, letters in the text), by default
/// ([`PlacePrices::localized`]). Measured in a lobby's playtest client (the op_cost probe's `TextLabel.Text write`: 56.6-62.2 us
/// for a five-letter text in three sessions, 71.7-75.5 for forty in two, against 0.7-0.8 with AutoLocalize off). An empty test
/// place has none (1.0 us for the same write), so a place not listed pays nothing. A place_size_writes probe ("a" / "b" written to
/// a TextLabel in a playtest client, medians of five sessions) read 56-59 us a write in the same experience's match places (the
/// lobby 56 for the same one-letter text): a localization table belongs to the experience, not to a place, so every place of an
/// experience with one is listed. A test place's playtest client paid 32-47 us (its own table); its edit DataModel, which the
/// 1.0 us above was read in, pays none, and ree's place 0 stays priced as that. `(place id, us of the lookup for a one-letter
/// text)`: the lobby 56.1, the match places 61.2 (five sessions, median: [`LOCALIZATION_LOOKUP`]).
pub const LOCALIZED_PLACES: &[(f64, f64)] = &[];

/// The lookup of a one-letter text, in microseconds, of a localized place given without its own figure (`REE_LOCALIZED_PLACES`
/// `<place id>`): the match places' median.
pub const LOCALIZATION_LOOKUP: f64 = 61.2;

/// The lookup a changed Text write adds in `place_id` (None: the place has no localization table in `prices`).
pub fn text_localization_in(prices: &PlacePrices, place_id: f64, characters: usize) -> Option<f64> {
    let (_, one_letter) = prices.localized.iter().find(|(place, _)| *place == place_id && place_id != 0.0)?;
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
/// A property write by what Studio does for it (client): a part's geometry in the workspace is dear (CFrame
/// and Position 1.10 us, Size 0.85, Color 0.55; out of the DataModel CFrame 0.16), a transparency or a UI
/// property 0.25-0.34, a joint's C0 0.25, Name 0.79, a Bone's or Motor6D's Transform 0.06. Nothing changed:
/// [`WRITE_QUIET`]. `placed` is whether the instance is in the workspace.
pub fn write(reflection: &Reflection, class: ClassId, property: &str, changed: bool, placed: bool) -> f64 {
    if !changed {
        return WRITE_QUIET;
    }
    if property == "Name" {
        return 0.79e-6;
    }
    let part = reflection.is_a(class, "BasePart");
    if part {
        return 1e-6
            * match property {
                "CFrame" | "Position" | "Orientation" | "Rotation" | "PivotOffset" => {
                    if placed { 1.10 } else { 0.165 }
                }
                "Size" => {
                    if placed { 0.85 } else { 0.17 }
                }
                "Color" | "BrickColor" | "Material" => 0.55,
                "Transparency" | "LocalTransparencyModifier" => 0.25,
                _ => 0.28,
            };
    }
    if property == "Transform" && (reflection.is_a(class, "Motor6D") || reflection.is_a(class, "Bone")) {
        return 0.06e-6;
    }
    if (property == "C0" || property == "C1") && reflection.is_a(class, "JointInstance") {
        return 0.25e-6;
    }
    if property == "Text" && matches!(&*reflection.class(class).name, "TextLabel" | "TextButton" | "TextBox") {
        return TEXT_WRITE;
    }
    0.30e-6
}

/// Instance.new by class (client and edit: a Part 1.1-1.3 us, a Folder or Attachment 0.4-0.5, a Frame 1.1-1.5, a
/// TextLabel 2.9-3.2; the others are Instance.new + Destroy less [`DESTROYED`] in the edit place).
pub fn created(reflection: &Reflection, class: ClassId) -> f64 {
    1e-6 * match &*reflection.class(class).name {
        "Humanoid" => 4.3,
        "TextLabel" | "TextButton" | "TextBox" => 2.9,
        "MeshPart" => 2.2,
        "BillboardGui" | "ScreenGui" => 1.9,
        "Animator" => 1.6,
        "Motor6D" | "ImageLabel" | "ImageButton" | "CanvasGroup" | "ScrollingFrame" | "UIListLayout" => 1.4,
        "SurfaceAppearance" => 1.2,
        _ if reflection.is_a(class, "BasePart") => 1.3,
        _ if reflection.is_a(class, "GuiObject") => 1.3,
        _ if reflection.is_a(class, "Constraint") => 0.35,
        _ => 0.5,
    }
}

/// An instance copied by Clone (edit, a Model of 50: the copy and its Destroy per instance less [`DESTROYED`]: a
/// Part 4.2 us (3.4-4.9 by the copy's size), MeshPart 5.4, Humanoid 9.5, TextButton 6.5, ScrollingFrame 5.6, ImageLabel
/// 3.6, Model / Motor6D / ParticleEmitter 2.0-2.7, Decal / UIGradient / UIStroke / UICorner / Beam / Trail / Highlight
/// 1.7-1.9, the Value classes, WeldConstraint, lights 1.2-1.3).
pub fn cloned(reflection: &Reflection, class: ClassId) -> f64 {
    1e-6 * match &*reflection.class(class).name {
        "Humanoid" => 9.5,
        "TextLabel" | "TextButton" | "TextBox" => 6.5,
        "MeshPart" => 5.4,
        "ScrollingFrame" => 5.6,
        "ImageLabel" | "ImageButton" => 3.6,
        "Model" | "ParticleEmitter" => 2.7,
        "Motor6D" => 2.3,
        "UIListLayout" | "ScreenGui" | "CanvasGroup" | "BillboardGui" | "SurfaceGui" => 3.3,
        "Sound" => 2.5,
        _ if reflection.is_a(class, "BasePart") => 4.2,
        _ if reflection.is_a(class, "GuiObject") => 3.5,
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
        "Fire" => 0.045 + 0.10,
        // (client: IsA 35 ns, IsDescendantOf 46, GetFullName 100 against the 62 of a call the table does not name)
        "IsA" => 0.0,
        "IsDescendantOf" | "IsAncestorOf" => 0.015,
        "GetFullName" => 0.080,
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

/// An instance of a subtree entering or leaving the DataModel (playtest: a grass patch of a
/// MeshPart and two children 11.5 us into a Folder in Workspace; capture, anchored: a Part or
/// MeshPart 9.7-14.0 us in, 7.6-9.5 out, a Folder 5.0-7.1 in, 2.7-4.0 out). An unanchored part
/// resting on others costs far more to take out (capture: 0.5-1.3 ms), which is not modelled.
pub fn moved(reflection: &Reflection, class: ClassId) -> f64 {
    if reflection.is_a(class, "BasePart") { 6e-6 } else { 3e-6 }
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

/// What the Luau VM did since it started (vendor/luau0-src, patches/luau.patch: lua_workcounters)
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
}

unsafe extern "C-unwind" {
    pub fn lua_workcounters(state: *mut mlua::ffi::lua_State) -> *const VmCounters;
    fn lua_setworkexempt(state: *mut mlua::ffi::lua_State, index: std::ffi::c_int);
    fn lua_workmute(state: *mut mlua::ffi::lua_State, on: std::ffi::c_int);
    fn lua_setgcclock(state: *mut mlua::ffi::lua_State, clock: Option<unsafe extern "C" fn(*mut std::ffi::c_void) -> f64>, ud: *mut std::ffi::c_void);
}

/// The garbage collector's clock: the scripts' own Luau work so far, in seconds of Studio's time ([`luau`] of the VM's counters).
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
    luau(unsafe { &*clock.counters })
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

/// Marks the engine's own Luau (the prelude, services, vector3.luau, record wrappers): Studio runs
/// that work in C++, so its instructions and what it allocates are not charged as a script's; a
/// script's call into it is ([`LUAU_HOSTCALL`]).
pub fn engine_luau(lua: &mlua::Lua, function: mlua::Function) -> mlua::Result<mlua::Function> {
    unsafe { lua.exec_raw::<()>(&function, |state| lua_setworkexempt(state, -1))? };
    Ok(function)
}

/// The Studio seconds scripts' own Luau work so far cost. Measured with os.clock over 46 workloads
/// (tools/luau-cost: loops of arithmetic, table reads and writes, calls, methods, constructors,
/// closures, builtins, generic and ipairs loops, Vector3 / CFrame / Color3 math, and seven workloads
/// of a game's own code: an effect's ground-wave fill and outer radius, object, registry and mesh-slice
/// loops) in a capture Studio's playtest client (a LocalScript, median of 7) against these counts
/// of the same code: a weighted fit, rounded (tools/luau-cost/fit.py), puts 31 of 39 within
/// 0.6-1.6x (cf.Position-style reads 2.8x and typeof 1.9x; CFrame * CFrame and a concatenated key's
/// lookup 0.3x, other CFrame / Color3 constructors 0.5x), not counting library calls
/// (string.format, string.find, .. on numbers, table.sort / concat / clone are 2-10x dearer, not
/// modelled: the game's preparation code hardly calls them), and the game's code within 0.5-1.2x
/// (the wave fill 349 us against Studio's 327, its 204,000 instructions a fill; the game's seven
/// workloads together 432 us against 443).
pub fn luau(c: &VmCounters) -> f64 {
    let ops: u64 = c.ops.iter().sum();
    let calls = c.ops[LOP_NAMECALL] + c.ops[LOP_CALL];
    let fastcalls: u64 = LOP_FASTCALLS.iter().map(|&op| c.ops[op]).sum();
    1e-9 * (ops as f64 * LUAU_OP
        + calls as f64 * LUAU_CALL
        + fastcalls as f64 * LUAU_FASTCALL
        + c.objects as f64 * LUAU_OBJECT
        + c.stored as f64 * LUAU_STORED
        + c.rehashed as f64 * LUAU_REHASHED
        + c.ccalls as f64 * LUAU_CCALL
        + c.hostcalls as f64 * LUAU_HOSTCALL
        + c.hostnanos as f64
        + c.vectormagnitudes as f64 * VECTOR_MAGNITUDE
        + c.vectorunits as f64 * VECTOR_UNIT
        + c.vectormethods as f64 * VECTOR_METHOD)
}

const LOP_NAMECALL: usize = 20;
const LOP_CALL: usize = 21;
const LOP_FASTCALLS: [usize; 5] = [60, 68, 73, 74, 75];

/// Nanoseconds: any instruction (a bare numeric loop 2.4 a step, arithmetic and table reads 1.4-1.9)
pub const LUAU_OP: f64 = 1.6;
/// a CALL or NAMECALL beyond its instruction (a Luau function call and return 17 a loop step, a method 24)
pub const LUAU_CALL: f64 = 8.5;
/// a builtin's fast call (math.floor / sin / max 22 for three)
pub const LUAU_FASTCALL: f64 = 2.3;
/// an object a script's instruction makes ({} 36, a closure 29, a record of three 54)
pub const LUAU_OBJECT: f64 = 22.0;
/// a byte of table storage (array and hash parts)
pub const LUAU_STORED: f64 = 0.4;
/// a key moved when a table grows (table.insert 38 a call as its list grows)
pub const LUAU_REHASHED: f64 = 12.0;
/// a C function call (the Instance API's own costs come on top: READ, CALL, ...)
pub const LUAU_CCALL: f64 = 5.0;
/// a datatype's operation the engine's Luau serves (CFrame * CFrame 62, CFrame.new 64, Color3.new 61,
/// cf * v 18, cf.Position + cf.LookVector 15, v.Magnitude / Unit / Dot 62 for three)
pub const LUAU_HOSTCALL: f64 = 15.0;
/// `.Magnitude` and `.Unit` on a Vector3, which the VM answers itself (patches/luau.patch, luaV_vectormember):
/// Studio's client reads `local _ = a.Magnitude` at 10.4 ns and `a.Unit` at 12.3 (the op_cost probe's rows, minus the
/// 3.2 ns of instructions the loop line counts). The rig frame, where a script reads it from a vector it just took from an
/// instance, is dearer (a match place's client, six runs of six readings: median 15.4 ns whole, 14.2 in the first three, range
/// 0-62 with the cold read of the vector's source), so it is priced 12.5, which reads about 14 here: 1.1x the rig frame, 1.2x the loop.
pub const VECTOR_MAGNITUDE: f64 = 12.5;
pub const VECTOR_UNIT: f64 = 9.1;
/// A method call on a Vector3 (`a:Dot(b)`): the NAMECALL and CALL charges above are fitted on Luau function calls, and
/// Studio's `Dot`, `Cross` and `Max` cost 9.4-12.2 ns whole where a C builtin called that way reads 18 (8.5 + 5 + the
/// instructions); negative, a correction to what they count
pub const VECTOR_METHOD: f64 = -9.0;
/// `Vector3:Lerp` and `:FuzzyEq` are still Luau functions of the engine (a host call each, src/lua/vector3.luau), which counted 19.3
/// ns where Studio's rig frame (the bridge_ops probe, a match place's client, three runs, six readings each) reads 31.2 and 35.9
/// (medians; the lobby client's op_cost loop 13.3 and 13.5): charged on top, in seconds as the other `R.Charge` figures
pub const VECTOR_LERP: f64 = 12.0e-9;
pub const VECTOR_FUZZY_EQ: f64 = 16.5e-9;
