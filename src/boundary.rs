//! The boundary profiler (`ree run ... --profile boundary`): every crossing from a script's Luau into the engine,
//! counted per call site (script, function, line) with the cost the work clock (cost.rs) charged for it.
//!
//! Roblox's Luau-to-C++ boundary is dear (a property read is 30-300 ns, a method 50-100 ns, a first touch of an instance
//! in a frame 0.3 us more), so code that crosses it every frame for a value that did not change is the cheapest kind of
//! win to find. The crossings counted: Instance property reads and writes by Class.Member, Instance methods, event
//! connects, Instance.new, and the datatype operations the VM counts as host calls (a CFrame, Color3 or UDim2
//! constructor, method or operator). A call site is the innermost frame of the game's own Luau; the engine's Luau
//! (`rur.*`, `ree.*`) is skipped, so a call into the prelude is the game line that made it.
//!
//! Gets also feed the repeat table: the same property of the same instance read again in one frame, by the same
//! function (`repeats`) or by another (`cross`), which is the signal that a cache removes the read.
//!
//! Off, the Instance API pays one thread-local read. On, a crossing walks the Luau stack to its call site (cached by
//! chunk, defining line and current line), so a run is several times slower than an unprofiled one; the counts are
//! exact for the paths below and the cost is the same charge the work clock takes, so the report is the same on every
//! run. Datatype operations are counted by the VM, not per call: each safepoint (a call, return or loop edge) books the
//! host calls and nanoseconds charged since the last one to the call site of the previous safepoint, so an operator
//! (`cf * cf`, which runs no call instruction) lands on the call before it, and a constructor cannot be told from
//! another by name. `workspace` and `game` are plain globals, not crossings. NOT counted: reads inside a muted engine call
//! (the engine's Luau pricing a whole method), Vector3 members, and what the engine does on its own (events delivered,
//! the physics and animation steps).

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    ffi::CStr,
    path::PathBuf,
};

use mlua::ffi;
use rustc_hash::{FxHashMap, FxHashSet};
use serde_json::{Value as Json, json};

/// What a crossing is
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[repr(u8)]
pub enum Kind {
    Get,
    Set,
    Method,
    Event,
    Create,
    Datatype,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Get => "get",
            Kind::Set => "set",
            Kind::Method => "method",
            Kind::Event => "event",
            Kind::Create => "create",
            Kind::Datatype => "datatype",
        }
    }
}

/// No instance (the repeat table skips it)
pub const NO_INSTANCE: u32 = u32::MAX;
/// A member the reflection has no name for (a child looked up by name)
pub const NO_MEMBER: u32 = u32::MAX;

#[derive(Clone, Debug)]
pub struct Config {
    /// Where the JSON report goes (`--profile-out`; default `ree-boundary.json` in the working directory)
    pub out: PathBuf,
    /// How many rows the printed report shows per table
    pub top: usize,
    /// The phases to report (`--profile-phase a,b` or `all`; default `steady` when the harness named one, else the one with the most frames)
    pub phase: Option<String>,
}

/// `--profile boundary [--profile-out <file>] [--profile-top <n>] [--profile-phase <name>]`, or the environment variable
/// `RUR_BOUNDARY_PROFILE=<file>`. None when neither is given.
pub fn parse(args: &[String]) -> Option<Config> {
    let value = |key: &str| args.iter().position(|arg| arg == key).and_then(|index| args.get(index + 1)).cloned();
    let flag = args.windows(2).any(|pair| pair[0] == "--profile" && pair[1] == "boundary");
    let env = std::env::var_os("RUR_BOUNDARY_PROFILE").filter(|path| !path.is_empty());
    if !flag && env.is_none() {
        return None;
    }
    let out = value("--profile-out").map(PathBuf::from).or_else(|| env.map(PathBuf::from)).unwrap_or_else(|| PathBuf::from("ree-boundary.json"));
    Some(Config { out, top: value("--profile-top").and_then(|top| top.parse().ok()).unwrap_or(30), phase: value("--profile-phase") })
}

/// Whether `--profile` in `args` is the timing profile (not `--profile boundary`)
pub fn timing_requested(args: &[String]) -> bool {
    args.iter().enumerate().any(|(index, arg)| arg == "--profile" && args.get(index + 1).map(String::as_str) != Some("boundary"))
}

thread_local! {
    static ACTIVE: Cell<bool> = const { Cell::new(false) };
    /// The Lua thread of the metamethod now running (set by `enter`, only while profiling)
    static CURRENT: Cell<*mut ffi::lua_State> = const { Cell::new(std::ptr::null_mut()) };
    static RECORDER: RefCell<Option<Recorder>> = const { RefCell::new(None) };
}

/// Whether this thread is profiling
#[inline]
pub fn active() -> bool {
    ACTIVE.with(Cell::get)
}

/// Called by the Instance metamethods and the engine's Lua callbacks on entry: the thread whose stack names the call site.
#[inline]
pub fn enter(state: *mut ffi::lua_State) {
    if active() {
        CURRENT.with(|current| current.set(state));
    }
}

#[derive(Default, Clone, Copy)]
struct Stat {
    count: u64,
    cost: f64,
    /// writes that changed nothing
    unchanged: u64,
    /// reads of a property already read this frame by the same function
    repeats: u64,
}

#[derive(Default, Clone, Copy)]
struct MemberStat {
    reads: u64,
    /// distinct (instance, member) pairs read in a frame, summed over the frames
    distinct: u64,
    same_function: u64,
    cross_function: u64,
}

#[derive(Default, Clone)]
struct Phase {
    name: String,
    seconds: f64,
    frames: u64,
    stats: FxHashMap<(u32, u32), Stat>,
    members: FxHashMap<u32, MemberStat>,
    /// seconds the Instance API charged, and the part of it a crossing accounts for
    charged: f64,
    attributed: f64,
    /// seconds charged for the engine delivering events to handlers (a handler thread resumed, a signal queued): not a crossing
    dispatched: f64,
}

struct Site {
    script: u32,
    function: u32,
    line: i32,
}

struct Function {
    script: u32,
    defined: i32,
    name: String,
}

struct Member {
    kind: Kind,
    class: String,
    member: String,
}

struct Recorder {
    config: Config,
    scripts: Vec<String>,
    script_ids: FxHashMap<usize, u32>,
    sites: Vec<Site>,
    site_ids: FxHashMap<(usize, i32, i32), u32>,
    functions: Vec<Function>,
    function_ids: FxHashMap<(u32, i32), u32>,
    members: Vec<Member>,
    member_ids: FxHashMap<(u8, u32, u32), u32>,
    member_texts: HashMap<(u8, String, String), u32>,
    phases: Vec<Phase>,
    current: usize,
    /// reads this frame: (function, instance, member), and (instance, member) with the function that read it last
    seen_function: FxHashSet<(u32, u32, u32)>,
    seen_any: FxHashMap<(u32, u32), u32>,
    /// the last crossing, which a follow-up charge of the same call (a whole-call price) adds its cost to
    last: Option<(u32, u32)>,
    /// the VM's host-call counters at the last safepoint, and the call site that safepoint was at
    host: Option<(u64, i64)>,
    previous: Option<u32>,
    datatype: Option<u32>,
}

/// Starts profiling on this thread. `phase` names are the luau_profile ones (`startup`, `frames`, what the harness set).
pub fn begin(config: Config) {
    RECORDER.with(|recorder| {
        *recorder.borrow_mut() = Some(Recorder {
            config,
            scripts: Vec::new(),
            script_ids: FxHashMap::default(),
            sites: Vec::new(),
            site_ids: FxHashMap::default(),
            functions: Vec::new(),
            function_ids: FxHashMap::default(),
            members: Vec::new(),
            member_ids: FxHashMap::default(),
            member_texts: HashMap::new(),
            phases: vec![Phase { name: "startup".into(), ..Phase::default() }],
            current: 0,
            seen_function: FxHashSet::default(),
            seen_any: FxHashMap::default(),
            last: None,
            host: None,
            previous: None,
            datatype: None,
        })
    });
    ACTIVE.with(|active| active.set(true));
}

/// A frame begins (`delta` virtual seconds long): the repeat table starts over and the phase's clock advances.
pub fn begin_frame(delta: f64) {
    with(|recorder| {
        let phase = &mut recorder.phases[recorder.current];
        phase.seconds += delta;
        phase.frames += 1;
        recorder.seen_function.clear();
        recorder.seen_any.clear();
    });
}

/// Names the phase the crossings after this call belong to (luau_profile::set_phase)
pub fn set_phase(name: &str) {
    with(|recorder| {
        recorder.current = match recorder.phases.iter().position(|phase| phase.name == name) {
            Some(index) => index,
            None => {
                recorder.phases.push(Phase { name: name.into(), ..Phase::default() });
                recorder.phases.len() - 1
            }
        };
    });
}

#[inline]
fn with(body: impl FnOnce(&mut Recorder)) {
    if !active() {
        return;
    }
    RECORDER.with(|recorder| {
        if let Ok(mut recorder) = recorder.try_borrow_mut() {
            if let Some(recorder) = recorder.as_mut() {
                body(recorder);
            }
        }
    });
}

/// The Instance API charged `seconds` to the work clock (Api::charge, Scheduler::charge)
#[inline]
pub fn charged(seconds: f64) {
    if active() {
        with(|recorder| recorder.phases[recorder.current].charged += seconds);
    }
}

/// The engine charged `seconds` for delivering events to handlers (the scheduler resuming a handler's thread, a signal queued)
#[inline]
pub fn dispatched(seconds: f64) {
    if active() {
        with(|recorder| recorder.phases[recorder.current].dispatched += seconds);
    }
}

/// A crossing: `class` and `member` are the reflection's ids (or [`NO_MEMBER`]); `names` gives their text the first time
/// they are seen. `cost` is the charge the crossing took. `changed` is false for a write that stored what the property
/// held. Nothing is recorded when no game Luau is running (the engine's own work).
pub fn record(kind: Kind, class: u32, member: u32, instance: u32, cost: f64, changed: bool, names: impl FnOnce() -> (String, String)) {
    if !active() {
        return;
    }
    let state = CURRENT.with(Cell::get);
    with(|recorder| {
        let Some(site) = recorder.site(state) else { return };
        let member = match recorder.member_ids.get(&(kind as u8, class, member)) {
            Some(&id) => id,
            None => {
                let (class_name, member_name) = names();
                let id = recorder.members.len() as u32;
                recorder.members.push(Member { kind, class: class_name, member: member_name });
                recorder.member_ids.insert((kind as u8, class, member), id);
                id
            }
        };
        recorder.count(kind, site, member, instance, cost, changed);
    });
}

/// A crossing named by text (an event connect, a datatype): not hot enough to need ids.
pub fn record_named(kind: Kind, class: &str, member: &str, cost: f64) {
    if !active() {
        return;
    }
    let state = CURRENT.with(Cell::get);
    with(|recorder| {
        let Some(site) = recorder.site(state) else { return };
        let member = recorder.text_member(kind, class, member);
        recorder.count(kind, site, member, NO_INSTANCE, cost, true);
    });
}

/// More cost for the crossing just recorded (a method's whole-call price, a list's elements): added to it, not counted again.
pub fn add_cost(seconds: f64) {
    with(|recorder| {
        if let Some(key) = recorder.last {
            let phase = &mut recorder.phases[recorder.current];
            if let Some(stat) = phase.stats.get_mut(&key) {
                stat.cost += seconds;
                phase.attributed += seconds;
            }
        }
    });
}

/// Called from the VM interrupt (a call, return or loop edge): books the datatype operations the VM counted since the
/// last safepoint to the call site that safepoint was at, then notes this one.
pub fn safepoint(state: *mut ffi::lua_State) {
    with(|recorder| {
        let counters = unsafe { &*crate::cost::lua_workcounters(state) };
        let now = (counters.hostcalls, counters.hostnanos);
        let before = recorder.host.replace(now);
        if let (Some((calls, nanos)), Some(site)) = (before, recorder.previous) {
            if now.0 != calls || now.1 != nanos {
                let cost = ((now.0 - calls) as f64 * crate::cost::LUAU_HOSTCALL + (now.1 - nanos) as f64) * 1e-9;
                let member = match recorder.datatype {
                    Some(member) => member,
                    None => {
                        let member = recorder.text_member(Kind::Datatype, "VM", "host call (datatype op or engine function)");
                        recorder.datatype = Some(member);
                        member
                    }
                };
                recorder.count_n(Kind::Datatype, site, member, NO_INSTANCE, now.0 - calls, cost, true);
            }
        }
        recorder.previous = recorder.site(state);
    });
}

impl Recorder {
    fn text_member(&mut self, kind: Kind, class: &str, member: &str) -> u32 {
        let key = (kind as u8, class.to_string(), member.to_string());
        if let Some(&id) = self.member_texts.get(&key) {
            return id;
        }
        let id = self.members.len() as u32;
        self.members.push(Member { kind, class: class.into(), member: member.into() });
        self.member_texts.insert(key, id);
        id
    }

    fn count(&mut self, kind: Kind, site: u32, member: u32, instance: u32, cost: f64, changed: bool) {
        self.count_n(kind, site, member, instance, 1, cost, changed);
    }

    #[allow(clippy::too_many_arguments)]
    fn count_n(&mut self, kind: Kind, site: u32, member: u32, instance: u32, times: u64, cost: f64, changed: bool) {
        let function = self.sites[site as usize].function;
        let phase = &mut self.phases[self.current];
        let stat = phase.stats.entry((site, member)).or_default();
        stat.count += times;
        stat.cost += cost;
        if kind != Kind::Datatype {
            phase.attributed += cost;
        }
        if kind == Kind::Set && !changed {
            stat.unchanged += 1;
        }
        if kind == Kind::Get && instance != NO_INSTANCE {
            let extra = phase.members.entry(member).or_default();
            extra.reads += 1;
            let same = !self.seen_function.insert((function, instance, member));
            match self.seen_any.insert((instance, member), function) {
                None => extra.distinct += 1,
                Some(_) if same => {}
                Some(_) => extra.cross_function += 1,
            }
            if same {
                stat.repeats += 1;
                extra.same_function += 1;
            }
        }
        self.last = Some((site, member));
    }

    /// The call site (the innermost frame of the game's own Luau) of `state`'s stack
    fn site(&mut self, state: *mut ffi::lua_State) -> Option<u32> {
        // (a thread that is not running is a stale one: the crossing came from the engine, not from Luau)
        if state.is_null() || unsafe { ffi::lua_status(state) } != ffi::LUA_OK {
            return None;
        }
        unsafe {
            let mut debug: ffi::lua_Debug = std::mem::zeroed();
            let mut level = 0;
            while level < 40 && ffi::lua_getinfo(state, level, c"sl".as_ptr(), &mut debug) != 0 {
                let here = level;
                level += 1;
                if debug.short_src.is_null() || debug.source.is_null() || debug.currentline < 0 {
                    continue;
                }
                let key = (debug.source as usize, debug.linedefined, debug.currentline);
                if let Some(&site) = self.site_ids.get(&key) {
                    return Some(site);
                }
                let script = match self.script_ids.get(&(debug.source as usize)) {
                    Some(&script) => script,
                    None => {
                        let text = CStr::from_ptr(debug.short_src).to_string_lossy().into_owned();
                        let engine = text == "[C]" || text.starts_with("ree.") || text.starts_with("rur.") || text.contains("prelude");
                        // (the engine's own Luau is marked with an empty name; it never becomes a site)
                        let name = if engine { String::new() } else { text.strip_prefix("[string \"").and_then(|inner| inner.strip_suffix("\"]")).unwrap_or(&text).to_string() };
                        self.scripts.push(name);
                        let id = self.scripts.len() as u32 - 1;
                        self.script_ids.insert(debug.source as usize, id);
                        id
                    }
                };
                if self.scripts[script as usize].is_empty() {
                    continue;
                }
                // (the function's name is the caller's name for it: read once per call site)
                let mut named: ffi::lua_Debug = std::mem::zeroed();
                let name = if ffi::lua_getinfo(state, here, c"n".as_ptr(), &mut named) != 0 && !named.name.is_null() {
                    CStr::from_ptr(named.name).to_string_lossy().into_owned()
                } else {
                    String::new()
                };
                let function = match self.function_ids.get(&(script, debug.linedefined)) {
                    Some(&function) => function,
                    None => {
                        self.functions.push(Function { script, defined: debug.linedefined, name: name.clone() });
                        let id = self.functions.len() as u32 - 1;
                        self.function_ids.insert((script, debug.linedefined), id);
                        id
                    }
                };
                if self.functions[function as usize].name.is_empty() {
                    self.functions[function as usize].name = name.clone();
                }
                self.sites.push(Site { script, function, line: debug.currentline });
                let site = self.sites.len() as u32 - 1;
                self.site_ids.insert(key, site);
                return Some(site);
            }
        }
        None
    }

    fn function_label(&self, function: u32) -> String {
        let function = &self.functions[function as usize];
        let name = if function.name.is_empty() { "anonymous" } else { function.name.as_str() };
        format!("{name}:{}", function.defined)
    }

    /// The phase to report: the ones asked for (`--profile-phase a,b`, or `all`; their crossings and seconds added), else `steady`
    /// when the harness named one, else the one with the most frames
    fn reported(&self) -> Phase {
        let named = |name: &str| self.phases.iter().find(|phase| phase.name == name && phase.frames > 0);
        let Some(wanted) = self.config.phase.as_deref() else {
            return named("steady").or_else(|| self.phases.iter().max_by_key(|phase| phase.frames)).unwrap_or(&self.phases[0]).clone();
        };
        let names: Vec<&str> = wanted.split(',').map(str::trim).collect();
        let chosen: Vec<&Phase> = self.phases.iter().filter(|phase| names.contains(&"all") || names.contains(&phase.name.as_str())).collect();
        if chosen.is_empty() {
            return self.phases.iter().max_by_key(|phase| phase.frames).unwrap_or(&self.phases[0]).clone();
        }
        let label = if names.contains(&"all") {
            "all".to_string()
        } else if names.len() > 6 {
            format!("{} phases {}..{}", names.len(), names[0], names[names.len() - 1])
        } else {
            wanted.to_string()
        };
        let mut merged = Phase { name: label, ..Phase::default() };
        for phase in chosen {
            merged.seconds += phase.seconds;
            merged.frames += phase.frames;
            merged.charged += phase.charged;
            merged.attributed += phase.attributed;
            merged.dispatched += phase.dispatched;
            for (&key, stat) in &phase.stats {
                let into = merged.stats.entry(key).or_default();
                into.count += stat.count;
                into.cost += stat.cost;
                into.unchanged += stat.unchanged;
                into.repeats += stat.repeats;
            }
            for (&key, stat) in &phase.members {
                let into = merged.members.entry(key).or_default();
                into.reads += stat.reads;
                into.distinct += stat.distinct;
                into.same_function += stat.same_function;
                into.cross_function += stat.cross_function;
            }
        }
        merged
    }

    fn report(&self) -> (Json, String) {
        let phase = self.reported();
        let phase = &phase;
        // (a phase in which no virtual time passed, a boot that runs inside one frame: its figures are per run, not per second)
        let zero_time = phase.seconds <= 0.0;
        let seconds = if zero_time { 1.0 } else { phase.seconds };
        let member_name = |member: u32| {
            let member = &self.members[member as usize];
            format!("{}.{}", member.class, member.member)
        };
        let ms = |cost: f64| cost / seconds * 1e3;

        // (a member read through different classes of instance, Part.CFrame and MeshPart.CFrame, is one: BasePart.CFrame)
        let mut canonical: HashMap<(u8, &str, &str), u32> = HashMap::new();
        let canon: Vec<u32> = self.members.iter().enumerate().map(|(id, member)| *canonical.entry((member.kind as u8, &member.class, &member.member)).or_insert(id as u32)).collect();
        let mut merged: FxHashMap<(u32, u32), Stat> = FxHashMap::default();
        for (&(site, member), stat) in &phase.stats {
            let into = merged.entry((site, canon[member as usize])).or_default();
            into.count += stat.count;
            into.cost += stat.cost;
            into.unchanged += stat.unchanged;
            into.repeats += stat.repeats;
        }
        let mut extras: FxHashMap<u32, MemberStat> = FxHashMap::default();
        for (&member, stat) in &phase.members {
            let into = extras.entry(canon[member as usize]).or_default();
            into.reads += stat.reads;
            into.distinct += stat.distinct;
            into.same_function += stat.same_function;
            into.cross_function += stat.cross_function;
        }

        let all_sites: Vec<_> = merged.iter().map(|(&(site, member), stat)| (site, member, *stat)).collect();
        // (the VM's host calls, datatype operations and engine functions, are ranked apart from the crossings)
        let is_host = |member: u32| self.members[member as usize].kind == Kind::Datatype;
        let mut sites: Vec<_> = all_sites.iter().filter(|row| !is_host(row.1)).cloned().collect();
        let mut host_sites: Vec<_> = all_sites.iter().filter(|row| is_host(row.1)).cloned().collect();
        sites.sort_by(|a, b| b.2.count.cmp(&a.2.count).then(a.0.cmp(&b.0)));
        host_sites.sort_by(|a, b| b.2.cost.total_cmp(&a.2.cost).then(a.0.cmp(&b.0)));
        let site_json = |(site, member, stat): &(u32, u32, Stat)| {
            let info = &self.sites[*site as usize];
            json!({
                "script": self.scripts[info.script as usize],
                "function": self.function_label(info.function),
                "line": info.line,
                "kind": self.members[*member as usize].kind.label(),
                "member": member_name(*member),
                "count": stat.count,
                "per_s": stat.count as f64 / seconds,
                "cost_ms_per_s": ms(stat.cost),
                "unchanged_writes": stat.unchanged,
                "repeat_reads": stat.repeats,
                "repeat_reads_per_s": stat.repeats as f64 / seconds,
            })
        };
        let mut by_cost = sites.clone();
        by_cost.sort_by(|a, b| b.2.cost.total_cmp(&a.2.cost).then(a.0.cmp(&b.0)));

        let mut members: FxHashMap<u32, (u64, f64, u64)> = FxHashMap::default();
        let mut scripts: FxHashMap<u32, (u64, f64)> = FxHashMap::default();
        let mut functions: FxHashMap<u32, (u64, f64)> = FxHashMap::default();
        let (mut crossings, mut priced, mut host_calls, mut host_cost) = (0u64, 0.0, 0u64, 0.0);
        for (site, member, stat) in &all_sites {
            let info = &self.sites[*site as usize];
            let entry = members.entry(*member).or_default();
            entry.0 += stat.count;
            entry.1 += stat.cost;
            entry.2 += 1;
            if !is_host(*member) {
                for (table, key) in [(&mut scripts, info.script), (&mut functions, info.function)] {
                    let entry = table.entry(key).or_default();
                    entry.0 += stat.count;
                    entry.1 += stat.cost;
                }
            }
            // (the VM's host calls include the engine functions a script calls through the Instance API, which are already
            // crossings of their own: they are reported apart, not added to the crossings)
            if self.members[*member as usize].kind == Kind::Datatype {
                host_calls += stat.count;
                host_cost += stat.cost;
            } else {
                crossings += stat.count;
                priced += stat.cost;
            }
        }
        let mut member_rows: Vec<_> = members.into_iter().collect();
        member_rows.sort_by(|a, b| b.1.0.cmp(&a.1.0).then(a.0.cmp(&b.0)));
        let frames = phase.frames.max(1) as f64;
        let member_json = |(member, (count, cost, call_sites)): &(u32, (u64, f64, u64))| {
            let extra = extras.get(member).copied().unwrap_or_default();
            json!({
                "member": member_name(*member),
                "kind": self.members[*member as usize].kind.label(),
                "count": count,
                "per_s": *count as f64 / seconds,
                "cost_ms_per_s": ms(*cost),
                "call_sites": call_sites,
                "reads_per_frame": extra.reads as f64 / frames,
                "distinct_per_frame": extra.distinct as f64 / frames,
                "same_function_repeats_per_frame": extra.same_function as f64 / frames,
                "cross_function_repeats_per_frame": extra.cross_function as f64 / frames,
            })
        };
        let mut script_rows: Vec<_> = scripts.into_iter().collect();
        script_rows.sort_by(|a, b| b.1.0.cmp(&a.1.0).then(a.0.cmp(&b.0)));
        let mut function_rows: Vec<_> = functions.into_iter().collect();
        function_rows.sort_by(|a, b| b.1.0.cmp(&a.1.0).then(a.0.cmp(&b.0)));
        let mut repeats: Vec<_> = sites.iter().filter(|row| row.2.repeats > 0).cloned().collect();
        repeats.sort_by(|a, b| b.2.repeats.cmp(&a.2.repeats).then(a.0.cmp(&b.0)));

        let coverage = if phase.charged > 0.0 { phase.attributed / phase.charged } else { 1.0 };
        let report = json!({
            "version": 1,
            "phase": phase.name,
            "phases": self.phases.iter().map(|phase| {
                let (mut count, mut cost) = (0u64, 0.0);
                for (&(_, member), stat) in &phase.stats {
                    if self.members[member as usize].kind != Kind::Datatype {
                        count += stat.count;
                        cost += stat.cost;
                    }
                }
                let rate = |value: f64| if phase.seconds > 0.0 { json!(value / phase.seconds) } else { Json::Null };
                json!({"phase": phase.name, "seconds": phase.seconds, "frames": phase.frames, "crossings": count, "cost_ms": cost * 1e3, "crossings_per_s": rate(count as f64), "cost_ms_per_s": rate(cost * 1e3)})
            }).collect::<Vec<_>>(),
            "emulated_seconds": phase.seconds,
            "zero_time": zero_time,
            "frames": phase.frames,
            "crossings": crossings,
            "crossings_per_s": crossings as f64 / seconds,
            "cost_ms_per_s": ms(priced),
            "host_calls": host_calls,
            "host_calls_per_s": host_calls as f64 / seconds,
            "host_calls_cost_ms_per_s": ms(host_cost),
            "instance_api_charged_ms_per_s": ms(phase.charged),
            "event_dispatch_ms_per_s": ms(phase.dispatched),
            "attributed_share_of_instance_api_charges": coverage,
            "by_site_count": sites.iter().take(300).map(site_json).collect::<Vec<_>>(),
            "by_site_cost": by_cost.iter().take(300).map(site_json).collect::<Vec<_>>(),
            "by_site_host_calls": host_sites.iter().take(300).map(site_json).collect::<Vec<_>>(),
            "by_member": member_rows.iter().take(500).map(member_json).collect::<Vec<_>>(),
            "by_module": script_rows.iter().take(400).map(|(script, (count, cost))| json!({
                "script": self.scripts[*script as usize], "count": count, "per_s": *count as f64 / seconds, "cost_ms_per_s": ms(*cost),
            })).collect::<Vec<_>>(),
            "by_function": function_rows.iter().take(300).map(|(function, (count, cost))| {
                let info = &self.functions[*function as usize];
                json!({"script": self.scripts[info.script as usize], "function": self.function_label(*function), "count": count, "per_s": *count as f64 / seconds, "cost_ms_per_s": ms(*cost)})
            }).collect::<Vec<_>>(),
            "repeat_reads": repeats.iter().take(200).map(site_json).collect::<Vec<_>>(),
        });

        let top = self.config.top;
        let mut text = format!(
            "boundary profile: phase {} ({:.2} emulated s, {} frames{}): {} crossings ({:.0}/s), priced {:.2} ms/s; {:.0} % of the Instance API's {:.2} ms/s of charges attributed\n\
             VM host calls (datatype operations and engine functions): {:.0}/s, {:.2} ms/s; the engine delivering events to handlers: {:.2} ms/s (not a crossing)\n",
            phase.name,
            phase.seconds,
            phase.frames,
            if zero_time { "; no virtual time passed, so the rates are per run" } else { "" },
            crossings,
            crossings as f64 / seconds,
            ms(priced),
            coverage * 100.0,
            ms(phase.charged),
            host_calls as f64 / seconds,
            ms(host_cost),
            ms(phase.dispatched)
        );
        let table = |title: &str, rows: &[(u32, u32, Stat)], text: &mut String| {
            text.push_str(&format!("\n{title}\n{:>10} {:>10} {:>8}  {:<8} {:<44} {}\n", "per s", "ms/s", "repeats", "kind", "member", "script:line function"));
            for (site, member, stat) in rows.iter().take(top) {
                let info = &self.sites[*site as usize];
                text.push_str(&format!(
                    "{:>10.1} {:>10.3} {:>8.1}  {:<8} {:<44} {}:{} {}\n",
                    stat.count as f64 / seconds,
                    ms(stat.cost),
                    stat.repeats as f64 / seconds,
                    self.members[*member as usize].kind.label(),
                    member_name(*member),
                    self.scripts[info.script as usize],
                    info.line,
                    self.function_label(info.function)
                ));
            }
        };
        table("top call sites by crossings per emulated second", &sites, &mut text);
        table("top call sites by priced cost", &by_cost, &mut text);
        table("VM host calls (datatype operations and engine functions) by priced cost", &host_sites, &mut text);
        text.push_str(&format!("\nby Class.Member\n{:>10} {:>10} {:>9} {:>9} {:>9} {:>6}  {:<8} {}\n", "per s", "ms/s", "reads/fr", "distinct", "repeat/fr", "sites", "kind", "member"));
        for row in member_rows.iter().take(top) {
            let extra = extras.get(&row.0).copied().unwrap_or_default();
            text.push_str(&format!(
                "{:>10.1} {:>10.3} {:>9.1} {:>9.1} {:>9.1} {:>6}  {:<8} {}\n",
                row.1.0 as f64 / seconds,
                ms(row.1.1),
                extra.reads as f64 / frames,
                extra.distinct as f64 / frames,
                (extra.same_function + extra.cross_function) as f64 / frames,
                row.1.2,
                self.members[row.0 as usize].kind.label(),
                member_name(row.0)
            ));
        }
        text.push_str(&format!("\nby module\n{:>10} {:>10}  {}\n", "per s", "ms/s", "script"));
        for (script, (count, cost)) in script_rows.iter().take(top) {
            text.push_str(&format!("{:>10.1} {:>10.3}  {}\n", *count as f64 / seconds, ms(*cost), self.scripts[*script as usize]));
        }
        text.push_str(&format!("\nrepeat reads (the same property of the same instance read again in a frame by the same function: cache it)\n{:>10} {:>10}  {:<44} {}\n", "repeat/s", "of reads/s", "member", "script:line function"));
        for (site, member, stat) in repeats.iter().take(top) {
            let info = &self.sites[*site as usize];
            text.push_str(&format!(
                "{:>10.1} {:>10.1}  {:<44} {}:{} {}\n",
                stat.repeats as f64 / seconds,
                stat.count as f64 / seconds,
                member_name(*member),
                self.scripts[info.script as usize],
                info.line,
                self.function_label(info.function)
            ));
        }
        (report, text)
    }
}

/// Ends profiling: the report as text, and as JSON in the configured file. None when this thread was not profiling.
pub fn finish() -> Option<String> {
    ACTIVE.with(|active| active.set(false));
    let recorder = RECORDER.with(|recorder| recorder.borrow_mut().take())?;
    let (report, text) = recorder.report();
    match serde_json::to_string_pretty(&report) {
        Ok(json) => {
            if let Err(error) = std::fs::write(&recorder.config.out, json) {
                eprintln!("Writing boundary profile {}: {error}", recorder.config.out.display());
            }
        }
        Err(error) => eprintln!("Encoding boundary profile: {error}"),
    }
    Some(text)
}
