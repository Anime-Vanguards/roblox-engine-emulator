//! A profile's files (profiler/mod.rs): `profile.json` (everything, for tools and `ree profile diff`), `summary.txt`
//! (the short text an agent reads; also printed), folded stacks for flame graph tools (`work.folded`,
//! `instructions.folded`, `time.folded`, `alloc.folded`, `alloc.counts.folded`, `heap.folded`), `profile.speedscope.json`
//! and `report.html` (one self-contained page).

use std::{
    collections::{BTreeMap, HashMap},
    fmt::Write as _,
    path::Path,
};

use rustc_hash::{FxHashMap, FxHashSet};
use serde_json::{Value as Json, json};

use super::{Config, Mark, Row, Run, Source, caches::Counts, cpu, memory, vm};

/// What a profile collected
pub struct Input<'a> {
    pub config: &'a Config,
    pub run: &'a Run,
    pub elapsed: f64,
    pub vm: Option<&'a vm::Results>,
    pub allocations: Option<&'a memory::Allocations>,
    pub time: Option<(&'a memory::Stacks, &'a FxHashMap<(u32, u32), u64>, u64)>,
    pub phases: &'a [String],
    pub marks: &'a [Mark],
    pub rows: &'a [Row],
    pub snapshots: &'a [memory::Snapshot],
    pub collector: &'a memory::Collector,
    pub caches: &'a [(&'static str, &'static str, Counts)],
    pub cpu: &'a cpu::Report,
    pub sources: &'a FxHashMap<String, Source>,
    /// The work clock's reading when the profile was collected (the last phase's end)
    pub end_work: f64,
    pub boundary: Option<&'a str>,
    pub phase_timing: Option<&'a str>,
    pub warnings: &'a [String],
}

/// The engine's own steps get function ids of their own (below every C function's)
const ENGINE_STEP_BASE: i32 = -1_000_000_000;

thread_local! {
    static ENGINE_STEPS: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// The function id of an engine step's frame in host time samples
pub fn engine_step_function(name: &str) -> i32 {
    ENGINE_STEPS.with(|steps| {
        let mut steps = steps.borrow_mut();
        let index = steps.iter().position(|known| known == name).unwrap_or_else(|| {
            steps.push(name.to_string());
            steps.len() - 1
        });
        ENGINE_STEP_BASE - index as i32
    })
}

/// A chunk's name as a module: `[string "ReplicatedStorage.X"]` is `ReplicatedStorage.X`
pub fn module_name(source: &str) -> &str {
    source.strip_prefix("[string \"").and_then(|inner| inner.strip_suffix("\"]")).or_else(|| source.strip_prefix('=')).or_else(|| source.strip_prefix('@')).unwrap_or(source)
}

/// The last segment of a module's name (`Grid` of `ReplicatedStorage.Interface.Grid`)
pub fn short_module(module: &str) -> &str {
    let module = module.trim_end_matches(".luau").trim_end_matches(".lua");
    module.rsplit(['.', '/', '\\']).next().unwrap_or(module)
}

/// Names of functions by id: Luau functions, C functions and the engine's steps
struct Names<'a> {
    vm: Option<&'a vm::Results>,
    steps: Vec<String>,
}

impl Names<'_> {
    fn function(&self, id: i32) -> Option<&vm::Function> {
        (id >= 0).then(|| self.vm.and_then(|vm| vm.functions.get(id as usize))).flatten()
    }

    /// `name module:line` (folded stacks, speedscope)
    fn frame(&self, id: i32) -> String {
        if id >= 0 {
            return match self.function(id) {
                Some(f) => format!("{} {}:{}", if f.name.is_empty() { "anonymous" } else { &f.name }, module_name(&f.source), f.line),
                None => format!("function #{id}"),
            };
        }
        if id <= ENGINE_STEP_BASE {
            return format!("(ree engine step: {})", self.steps.get((ENGINE_STEP_BASE - id) as usize).map_or("?", String::as_str));
        }
        let name = self.vm.and_then(|vm| vm.cfunctions.get((-2 - id) as usize)).map_or("", String::as_str);
        format!("{} [C]", if name.is_empty() { "?" } else { name })
    }

    /// `name  Module:line` (tables)
    fn label(&self, id: i32) -> String {
        match self.function(id) {
            Some(f) => format!("{}  {}:{}", if f.name.is_empty() { "anonymous" } else { &f.name }, short_module(module_name(&f.source)), f.line),
            None => self.frame(id),
        }
    }

    fn engine(&self, id: i32) -> bool {
        self.function(id).is_some_and(vm::Function::exempt)
    }
}

/// Per-function figures
#[derive(Clone, Copy, Debug, Default)]
struct Figures {
    calls: u64,
    self_insns: u64,
    total_insns: u64,
    self_work: f64,
    total_work: f64,
    self_time: f64,
    total_time: f64,
    self_alloc: f64,
    total_alloc: f64,
    self_allocs: f64,
    total_allocs: f64,
}

/// Per-line figures (a function's line)
#[derive(Clone, Debug, Default)]
struct LineFigures {
    insns: u64,
    work: f64,
    total_work: f64,
    total_insns: u64,
    time: f64,
    total_time: f64,
    alloc: f64,
    allocs: f64,
    ops: BTreeMap<u8, u64>,
}

struct Model {
    functions: FxHashMap<i32, Figures>,
    lines: FxHashMap<(i32, i32), LineFigures>,
    /// (caller, callee) -> (calls, total work, total instructions)
    edges: FxHashMap<(i32, i32), (u64, f64, u64)>,
    /// Per-node subtree totals (work, instructions)
    subtree: Vec<(f64, u64)>,
    /// The phase each node is under
    node_phase: Vec<u32>,
}

/// The call tree's per-function and per-line figures, and its edges
fn tree_model(vm: &vm::Results) -> Model {
    let nodes = &vm.nodes;
    let mut model = Model { functions: FxHashMap::default(), lines: FxHashMap::default(), edges: FxHashMap::default(), subtree: vec![(0.0, 0); nodes.len()], node_phase: vec![0; nodes.len()] };
    // phases: a node under the root with function -1 is a phase's
    for (index, node) in nodes.iter().enumerate() {
        model.node_phase[index] = if node.function == -1 && node.parent == 0 && index != 0 {
            node.callpc.max(0) as u32
        } else if node.parent >= 0 {
            model.node_phase[node.parent as usize]
        } else {
            0
        };
    }
    for index in (0..nodes.len()).rev() {
        let node = &nodes[index];
        model.subtree[index].0 += node.work;
        model.subtree[index].1 += node.insns;
        if node.parent >= 0 {
            let (work, insns) = model.subtree[index];
            model.subtree[node.parent as usize].0 += work;
            model.subtree[node.parent as usize].1 += insns;
        }
    }
    let real = |id: i32| id >= 0 || id < -1;
    for (index, node) in nodes.iter().enumerate() {
        if !real(node.function) {
            continue;
        }
        let figures = model.functions.entry(node.function).or_default();
        figures.calls += node.calls;
        figures.self_insns += node.insns;
        figures.self_work += node.work;
        // total: only the outermost node of a function on its path (recursion counts once)
        let mut up = node.parent;
        let mut nested = false;
        while up >= 0 {
            if nodes[up as usize].function == node.function {
                nested = true;
                break;
            }
            up = nodes[up as usize].parent;
        }
        if !nested {
            figures.total_work += model.subtree[index].0;
            figures.total_insns += model.subtree[index].1;
        }
        if node.parent >= 0 {
            let parent = &nodes[node.parent as usize];
            if real(parent.function) {
                let edge = model.edges.entry((parent.function, node.function)).or_default();
                edge.0 += node.calls;
                edge.1 += model.subtree[index].0;
                edge.2 += model.subtree[index].1;
                // the calling line's total
                if parent.function >= 0 && node.callpc >= 0 {
                    if let Some(line) = vm.functions.get(parent.function as usize).and_then(|f| f.lines.get(node.callpc as usize)) {
                        let figures = model.lines.entry((parent.function, *line)).or_default();
                        figures.total_work += model.subtree[index].0;
                        figures.total_insns += model.subtree[index].1;
                    }
                }
            }
        }
    }
    for (id, function) in vm.functions.iter().enumerate() {
        for (pc, &count) in function.counts.iter().enumerate() {
            let work = function.work.get(pc).copied().unwrap_or(0.0);
            if count == 0 && work == 0.0 {
                continue;
            }
            let line = function.lines.get(pc).copied().unwrap_or(0);
            let figures = model.lines.entry((id as i32, line)).or_default();
            figures.insns += count;
            figures.work += work;
            figures.total_work += work;
            figures.total_insns += count;
            if count > 0 {
                *figures.ops.entry((function.code[pc] & 0xff) as u8).or_default() += count;
            }
        }
    }
    model
}

/// Stack samples (innermost first) folded into per-function and per-line self and total figures
fn add_samples(model: &mut Model, stack: &[memory::Frame], weight: f64, field: fn(&mut Figures, bool) -> &mut f64, line_field: fn(&mut LineFigures, bool) -> &mut f64, luau_self: bool) {
    // self: the innermost frame (or, for allocations, the innermost Luau frame: the line that called a C function)
    let innermost = if luau_self { stack.iter().find(|frame| frame.0 >= 0).or(stack.first()) } else { stack.first() };
    if let Some(&(function, line)) = innermost {
        *field(model.functions.entry(function).or_default(), true) += weight;
        if function >= 0 {
            *line_field(model.lines.entry((function, line)).or_default(), true) += weight;
        }
    }
    let mut seen_functions: FxHashSet<i32> = FxHashSet::default();
    let mut seen_lines: FxHashSet<(i32, i32)> = FxHashSet::default();
    for &(function, line) in stack {
        if seen_functions.insert(function) {
            *field(model.functions.entry(function).or_default(), false) += weight;
        }
        if function >= 0 && seen_lines.insert((function, line)) {
            *line_field(model.lines.entry((function, line)).or_default(), false) += weight;
        }
    }
}

fn ms(seconds: f64) -> f64 {
    seconds * 1e3
}

fn round(value: f64, digits: i32) -> f64 {
    let scale = 10f64.powi(digits);
    (value * scale).round() / scale
}

fn group_digits(value: u64) -> String {
    let text = value.to_string();
    let mut out = String::new();
    for (index, ch) in text.chars().enumerate() {
        if index > 0 && (text.len() - index) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

fn bytes(value: f64) -> String {
    let value = value.max(0.0);
    if value >= 1024.0 * 1024.0 {
        format!("{:.2} MB", value / (1024.0 * 1024.0))
    } else if value >= 1024.0 {
        format!("{:.1} KB", value / 1024.0)
    } else {
        format!("{value:.0} B")
    }
}

fn percent(part: f64, whole: f64) -> String {
    if whole <= 0.0 { "-".into() } else { format!("{:.1} %", part / whole * 100.0) }
}

/// The source text of a function's lines (for the annotated listing): (line, text) from its first to its last line
fn listing(input: &Input, function: &vm::Function, model: &Model, id: i32) -> Option<(String, Vec<(i32, String)>)> {
    let module = module_name(&function.source);
    let source = input.sources.get(module)?;
    let text = String::from_utf8_lossy(&source.text);
    let lines: Vec<&str> = text.lines().collect();
    let first = function.line.max(1);
    let last = function.lines.iter().copied().max().unwrap_or(first).max(first);
    // (the lines the model holds for this function, so the listing shows each with its figures)
    let _ = (model, id);
    let rows = (first..=last.min(first + 400)).filter_map(|line| lines.get(line as usize - 1).map(|text| (line, text.to_string()))).collect();
    Some((source.path.clone().unwrap_or_else(|| module.to_string()), rows))
}

/// Writes every file of the profile and returns the summary text
pub fn write(input: &Input) -> Result<String, String> {
    let out = &input.config.out;
    std::fs::create_dir_all(out).map_err(|error| format!("--profile-out {}: {error}", out.display()))?;
    let names = Names { vm: input.vm, steps: ENGINE_STEPS.with(|steps| steps.borrow().clone()) };
    let mut model = input.vm.map(tree_model).unwrap_or(Model { functions: FxHashMap::default(), lines: FxHashMap::default(), edges: FxHashMap::default(), subtree: Vec::new(), node_phase: Vec::new() });

    // host time samples (ms) and allocations (bytes) into the same model
    let tick = 1e-3;
    let mut time_total = 0.0;
    if let Some((stacks, samples, _)) = input.time {
        for (&(_, stack), &ticks) in samples {
            let weight = ticks as f64 * tick;
            time_total += weight;
            add_samples(&mut model, &stacks.stacks[stack as usize], weight, |f, own| if own { &mut f.self_time } else { &mut f.total_time }, |l, own| if own { &mut l.time } else { &mut l.total_time }, false);
        }
    }
    if let Some(allocations) = input.allocations {
        for (&(_, stack, _), amount) in &allocations.by_stack {
            let frames = &allocations.stacks.stacks[stack as usize];
            add_samples(&mut model, frames, amount.bytes, |f, own| if own { &mut f.self_alloc } else { &mut f.total_alloc }, |l, _| &mut l.alloc, true);
            add_samples(&mut model, frames, amount.count, |f, own| if own { &mut f.self_allocs } else { &mut f.total_allocs }, |l, _| &mut l.allocs, true);
        }
    }

    let seconds = input.run.seconds.max(0.0);
    let frames = input.run.frames.max(1) as f64;
    let mut files: Vec<String> = Vec::new();
    let mut json = json!({
        "version": 1,
        "tool": "ree",
        "kinds": input.config.kinds.names(),
        "run": {
            "command": input.run.command, "status": input.run.status, "harness": input.run.harness, "frames": input.run.frames,
            "seconds": input.run.seconds, "fps": input.run.fps, "elapsed": input.elapsed,
        },
        "phases": input.phases,
        "marks": input.marks.iter().map(|mark| json!({"phase": mark.phase, "frame": mark.frame, "seconds": mark.seconds, "work_ms": ms(mark.work)})).collect::<Vec<_>>(),
        "warnings": input.warnings,
    });

    // functions: every one with figures, keyed so a diff finds it again
    let mut function_rows: Vec<(i32, Figures)> = model.functions.iter().map(|(&id, figures)| (id, *figures)).collect();
    function_rows.sort_by(|a, b| b.1.self_work.total_cmp(&a.1.self_work).then(b.1.total_time.total_cmp(&a.1.total_time)).then(b.1.total_alloc.total_cmp(&a.1.total_alloc)).then(a.0.cmp(&b.0)));
    let function_json = |id: i32, figures: &Figures| -> Json {
        let function = names.function(id);
        json!({
            "id": id,
            "key": function_key(&names, id),
            "name": function.map_or_else(|| names.frame(id), |f| f.name.clone()),
            "module": function.map(|f| module_name(&f.source).to_string()),
            "line": function.map(|f| f.line),
            "last_line": function.and_then(|f| f.lines.iter().copied().max()),
            "kind": match function { Some(f) if f.exempt() => "engine Luau", Some(_) => "Luau", None if id <= ENGINE_STEP_BASE => "engine step", None => "C" },
            "native": function.is_some_and(vm::Function::native),
            "native_entries": function.map_or(0, |f| f.native_entries),
            "native_work_ms": function.map_or(0.0, |f| ms(f.native_work)),
            "calls": figures.calls,
            "self": {"insns": figures.self_insns, "work_ms": ms(figures.self_work), "time_ms": figures.self_time, "alloc_bytes": figures.self_alloc.round(), "allocs": figures.self_allocs.round()},
            "total": {"insns": figures.total_insns, "work_ms": ms(figures.total_work), "time_ms": figures.total_time, "alloc_bytes": figures.total_alloc.round(), "allocs": figures.total_allocs.round()},
        })
    };
    json["functions"] = Json::Array(function_rows.iter().map(|(id, figures)| function_json(*id, figures)).collect());

    // lines (with their opcodes)
    let mut line_rows: Vec<(&(i32, i32), &LineFigures)> = model.lines.iter().collect();
    line_rows.sort_by(|a, b| b.1.work.total_cmp(&a.1.work).then(b.1.time.total_cmp(&a.1.time)).then(b.1.alloc.total_cmp(&a.1.alloc)).then(a.0.cmp(b.0)));
    json["lines"] = Json::Array(
        line_rows
            .iter()
            .map(|((function, line), figures)| {
                json!({
                    "function": function, "line": line, "insns": figures.insns, "work_ms": ms(figures.work), "total_work_ms": ms(figures.total_work),
                    "total_insns": figures.total_insns, "time_ms": figures.time, "total_time_ms": figures.total_time, "alloc_bytes": figures.alloc.round(), "allocs": figures.allocs.round(),
                    "ops": figures.ops.iter().map(|(op, count)| (vm::opcode_name(*op).to_string(), json!(count))).collect::<serde_json::Map<_, _>>(),
                })
            })
            .collect(),
    );
    json["edges"] = Json::Array({
        let mut edges: Vec<_> = model.edges.iter().collect();
        edges.sort_by(|a, b| b.1.1.total_cmp(&a.1.1).then(a.0.cmp(b.0)));
        edges.iter().map(|((caller, callee), (calls, work, insns))| json!({"caller": caller, "callee": callee, "calls": calls, "work_ms": ms(*work), "insns": insns})).collect()
    });

    // the call tree for the flame graph: [parent, function, calls, self work in us, self instructions, label]
    if let Some(vm_results) = input.vm {
        json["tree"] = Json::Array(
            vm_results
                .nodes
                .iter()
                .enumerate()
                .map(|(index, node)| {
                    let label = if index == 0 {
                        Some(input.phases.first().cloned().unwrap_or_else(|| "startup".into()))
                    } else if node.function == -1 {
                        Some(input.phases.get(node.callpc.max(0) as usize).cloned().unwrap_or_default())
                    } else if node.function < -1 {
                        Some(names.frame(node.function))
                    } else {
                        None
                    };
                    json!([node.parent, node.function, node.calls, (node.work * 1e6).round(), node.insns, label])
                })
                .collect(),
        );
    }

    let mut summary = String::new();
    let _ = writeln!(
        summary,
        "ree profile: {} {}; {} frames, {:.2} s emulated{}; kinds {}; {:.2} s wall",
        input.run.command,
        input.run.status,
        input.run.frames,
        seconds,
        if input.run.fps > 0.0 { format!(" at {} fps", input.run.fps) } else { String::new() },
        input.config.kinds.names().join(", "),
        input.elapsed
    );
    for warning in input.warnings {
        let _ = writeln!(summary, "warning: {warning}");
    }

    // work and instructions
    if let Some(vm_results) = input.vm.filter(|vm| !vm.nodes.is_empty() && input.config.kinds.steps()) {
        let total_work: f64 = vm_results.nodes.iter().map(|node| node.work).sum::<f64>() + vm_results.engine_work;
        let script_insns: u64 = vm_results.functions.iter().filter(|f| !f.exempt()).flat_map(|f| f.counts.iter()).sum();
        let engine_insns: u64 = vm_results.functions.iter().filter(|f| f.exempt()).flat_map(|f| f.counts.iter()).sum();
        let natives: Vec<&vm::Function> = vm_results.functions.iter().filter(|f| f.native() && !f.exempt()).collect();
        let mut opcodes: BTreeMap<u8, (u64, f64)> = BTreeMap::new();
        for function in vm_results.functions.iter().filter(|f| !f.exempt()) {
            for (pc, &count) in function.counts.iter().enumerate() {
                if count > 0 {
                    let entry = opcodes.entry((function.code[pc] & 0xff) as u8).or_default();
                    entry.0 += count;
                    entry.1 += function.work.get(pc).copied().unwrap_or(0.0);
                }
            }
        }
        let mut opcode_rows: Vec<_> = opcodes.into_iter().collect();
        opcode_rows.sort_by(|a, b| b.1.0.cmp(&a.1.0));
        json["work"] = json!({
            "total_ms": ms(total_work),
            "outside_instructions_ms": ms(vm_results.engine_work),
            "per_second_ms": if seconds > 0.0 { ms(total_work) / seconds } else { 0.0 },
            "per_frame_ms": ms(total_work) / frames,
            "script_insns": script_insns,
            "engine_insns": engine_insns,
            "native_functions": natives.iter().map(|f| json!({"name": f.name, "module": module_name(&f.source), "line": f.line, "entries": f.native_entries, "work_ms": ms(f.native_work)})).collect::<Vec<_>>(),
            "opcodes": opcode_rows.iter().map(|(op, (count, work))| json!({"op": vm::opcode_name(*op), "count": count, "work_ms": ms(*work)})).collect::<Vec<_>>(),
        });
        let _ = writeln!(
            summary,
            "work clock {:.2} ms of Studio time ({:.2} ms/s, {:.3} ms a frame); {:.2} ms charged outside any instruction (event dispatch, resumptions)",
            ms(total_work),
            if seconds > 0.0 { ms(total_work) / seconds } else { 0.0 },
            ms(total_work) / frames,
            ms(vm_results.engine_work)
        );
        let _ = writeln!(summary, "instructions: {} by scripts, {} by the engine's own Luau", group_digits(script_insns), group_digits(engine_insns));
        if natives.is_empty() {
            let _ = writeln!(summary, "natively compiled functions: none ran natively (every instruction is counted)");
        } else {
            let entries: u64 = natives.iter().map(|f| f.native_entries).sum();
            let work: f64 = natives.iter().map(|f| f.native_work).sum();
            let named: Vec<String> = natives.iter().take(6).map(|f| format!("{}:{}", short_module(module_name(&f.source)), f.line)).collect();
            let _ = writeln!(
                summary,
                "natively compiled functions: {} ran natively ({}{}; {} entries, {:.3} ms of work booked while they ran): their instructions are not counted, only their entries",
                natives.len(),
                named.join(", "),
                if natives.len() > named.len() { ", ..." } else { "" },
                group_digits(entries),
                ms(work)
            );
        }
        if !input.marks.is_empty() {
            // (each mark's work clock reading to the next one's, the last to the run's end)
            let mut parts = Vec::new();
            for (index, mark) in input.marks.iter().enumerate() {
                let end = input.marks.get(index + 1).map_or(input.end_work, |next| next.work);
                parts.push(format!("{} (frame {}) {:.2} ms", mark.phase, mark.frame, ms((end - mark.work).max(0.0))));
            }
            let _ = writeln!(summary, "phases' work: {}", parts.join("; "));
        }
        let top = input.config.top;
        let _ = writeln!(summary, "\ntop functions by self work (ms of Studio time; total includes callees)");
        let _ = writeln!(summary, "{:>9} {:>9} {:>10} {:>12}  function", "self ms", "total ms", "calls", "insns");
        // (rows that print as zero are left out)
        for (id, figures) in function_rows.iter().filter(|(id, figures)| figures.self_work >= 5e-7 && !names.engine(*id)).take(top) {
            let _ = writeln!(summary, "{:>9.3} {:>9.3} {:>10} {:>12}  {}", ms(figures.self_work), ms(figures.total_work), group_digits(figures.calls), group_digits(figures.self_insns), names.label(*id));
        }
        let mut by_total: Vec<&(i32, Figures)> = function_rows.iter().filter(|(id, _)| *id >= 0 && !names.engine(*id)).collect();
        by_total.sort_by(|a, b| b.1.total_work.total_cmp(&a.1.total_work));
        let _ = writeln!(summary, "\ntop functions by total work");
        let _ = writeln!(summary, "{:>9} {:>9} {:>10}  function", "total ms", "self ms", "calls");
        for (id, figures) in by_total.iter().filter(|(_, figures)| figures.total_work >= 5e-7).take(top.min(12)) {
            let _ = writeln!(summary, "{:>9.3} {:>9.3} {:>10}  {}", ms(figures.total_work), ms(figures.self_work), group_digits(figures.calls), names.label(*id));
        }
        let _ = writeln!(summary, "\ntop lines by self work");
        let _ = writeln!(summary, "{:>9} {:>11} {:>12}  line  (opcodes)", "self ms", "total ms", "insns");
        for ((function, line), figures) in line_rows.iter().filter(|(key, figures)| figures.work >= 5e-7 && !names.engine(key.0)).take(top) {
            let ops: Vec<String> = {
                let mut ops: Vec<_> = figures.ops.iter().collect();
                ops.sort_by(|a, b| b.1.cmp(a.1));
                ops.iter().take(4).map(|(op, count)| format!("{} {}", vm::opcode_name(**op), group_digits(**count))).collect()
            };
            let module = names.function(*function).map_or(String::new(), |f| short_module(module_name(&f.source)).to_string());
            let _ = writeln!(summary, "{:>9.3} {:>11.3} {:>12}  {module}:{line}  ({})", ms(figures.work), ms(figures.total_work), group_digits(figures.insns), ops.join(", "));
        }
        // inline caches
        if input.config.kinds.has(super::Kinds::CACHES) {
            let mut by_kind: BTreeMap<&str, [u64; 4]> = BTreeMap::new();
            let mut sites: Vec<(i32, usize, u8, [u64; 4])> = Vec::new();
            for (id, function) in vm_results.functions.iter().enumerate() {
                if function.exempt() || function.caches.is_empty() {
                    continue;
                }
                for pc in 0..function.code.len() {
                    let outcomes: [u64; 4] = std::array::from_fn(|slot| function.caches[pc * vm::CACHE_OUTCOMES + slot]);
                    if outcomes.iter().sum::<u64>() == 0 {
                        continue;
                    }
                    let op = (function.code[pc] & 0xff) as u8;
                    if let Some(kind) = vm::cache_kind(op) {
                        let entry = by_kind.entry(kind).or_default();
                        for slot in 0..4 {
                            entry[slot] += outcomes[slot];
                        }
                    }
                    sites.push((id as i32, pc, op, outcomes));
                }
            }
            let _ = writeln!(summary, "\nLuau inline caches (hit: the cached slot or import served it; miss: repredicted; slow: a metatable's path or an unsafe environment; other: not a table)");
            let _ = writeln!(summary, "{:<18} {:>8} {:>12} {:>10} {:>10} {:>10}", "cache", "hit", "runs", "misses", "slow", "other");
            for (kind, outcomes) in &by_kind {
                let runs: u64 = outcomes.iter().sum();
                let table = (outcomes[0] + outcomes[1] + outcomes[2]) as f64;
                let _ = writeln!(summary, "{:<18} {:>8} {:>12} {:>10} {:>10} {:>10}", kind, percent(outcomes[0] as f64, table), group_digits(runs), group_digits(outcomes[1]), group_digits(outcomes[2]), group_digits(outcomes[3]));
            }
            sites.sort_by(|a, b| (b.3[1] + b.3[2]).cmp(&(a.3[1] + a.3[2])).then(a.0.cmp(&b.0)).then(a.1.cmp(&b.1)));
            let worst: Vec<_> = sites.iter().filter(|site| site.3[1] + site.3[2] > 0).take(top.min(10)).collect();
            if !worst.is_empty() {
                let _ = writeln!(summary, "sites missing most");
                for (function, pc, op, outcomes) in worst {
                    let f = &vm_results.functions[*function as usize];
                    let _ = writeln!(summary, "  {:>10} misses {:>10} slow of {:>10}  {} {}:{}", group_digits(outcomes[1]), group_digits(outcomes[2]), group_digits(outcomes.iter().sum()), vm::opcode_name(*op), short_module(module_name(&f.source)), f.lines.get(*pc).copied().unwrap_or(0));
                }
            }
            json["luau_caches"] = json!({
                "by_kind": by_kind.iter().map(|(kind, o)| json!({"kind": kind, "hits": o[0], "misses": o[1], "slow": o[2], "other": o[3]})).collect::<Vec<_>>(),
                "sites": sites.iter().take(2000).map(|(function, pc, op, o)| {
                    let f = &vm_results.functions[*function as usize];
                    json!({"function": function, "line": f.lines.get(*pc).copied().unwrap_or(0), "pc": pc, "op": vm::opcode_name(*op), "hits": o[0], "misses": o[1], "slow": o[2], "other": o[3]})
                }).collect::<Vec<_>>(),
            });
        }
    }

    // host time
    if let Some((_, _, _)) = input.time {
        json["time"] = json!({"sampled_ms": time_total});
        let _ = writeln!(summary, "\nhost time sampled: {:.1} ms (1 ms ticks at the VM's calls, returns and loop edges; a line's time lands on its next safepoint)", time_total);
        let mut by_self: Vec<&(i32, Figures)> = function_rows.iter().filter(|(_, f)| f.self_time > 0.0).collect();
        by_self.sort_by(|a, b| b.1.self_time.total_cmp(&a.1.self_time));
        let _ = writeln!(summary, "{:>9} {:>9}  function", "self ms", "total ms");
        for (id, figures) in by_self.iter().take(input.config.top) {
            let _ = writeln!(summary, "{:>9.1} {:>9.1}  {}", figures.self_time, figures.total_time, names.label(*id));
        }
    }

    // allocations
    if let Some(allocations) = input.allocations {
        let mut by_type: Vec<(String, memory::Amount)> = allocations.types.names.iter().cloned().zip(allocations.by_type.iter().copied()).collect();
        by_type.sort_by(|a, b| b.1.bytes.total_cmp(&a.1.bytes));
        let mut groups: BTreeMap<String, memory::Amount> = BTreeMap::new();
        for (name, amount) in &by_type {
            let entry = groups.entry(memory::group(name).to_string()).or_default();
            entry.bytes += amount.bytes;
            entry.count += amount.count;
        }
        let mut groups: Vec<_> = groups.into_iter().collect();
        groups.sort_by(|a, b| b.1.bytes.total_cmp(&a.1.bytes));
        let total = allocations.bytes as f64;
        let _ = writeln!(
            summary,
            "\nallocations: {} in {} objects ({}){}",
            bytes(total),
            group_digits(allocations.objects),
            groups.iter().take(6).map(|(name, amount)| format!("{name} {}", percent(amount.bytes, total))).collect::<Vec<_>>().join(", "),
            if allocations.step > 1 { format!("; stacks sampled every {} bytes", allocations.step) } else { String::new() }
        );
        let mut stacks: FxHashMap<(u32, u32), memory::Amount> = FxHashMap::default();
        for (&(_, stack, ty), amount) in &allocations.by_stack {
            let entry = stacks.entry((stack, ty)).or_default();
            entry.bytes += amount.bytes;
            entry.count += amount.count;
        }
        let mut stack_rows: Vec<_> = stacks.into_iter().collect();
        stack_rows.sort_by(|a, b| b.1.bytes.total_cmp(&a.1.bytes));
        let describe = |stack: u32| -> String {
            let frames = &allocations.stacks.stacks[stack as usize];
            let luau: Vec<String> = frames.iter().filter(|frame| frame.0 >= 0 && !names.engine(frame.0)).take(3).map(|&(function, line)| {
                let module = names.function(function).map_or(String::new(), |f| short_module(module_name(&f.source)).to_string());
                format!("{module}:{line}")
            }).collect();
            let top_c = frames.first().filter(|frame| frame.0 < -1).map(|frame| format!("{} < ", names.frame(frame.0))).unwrap_or_default();
            format!("{top_c}{}", if luau.is_empty() { "(engine)".into() } else { luau.join(" < ") })
        };
        let _ = writeln!(summary, "{:>10} {:>10}  type  stack (innermost Luau lines first)", "bytes", "count");
        for ((stack, ty), amount) in stack_rows.iter().take(input.config.top) {
            let _ = writeln!(summary, "{:>10} {:>10}  {}  {}", bytes(amount.bytes), group_digits(amount.count.round() as u64), allocations.types.names[*ty as usize], describe(*stack));
        }
        json["alloc"] = json!({
            "bytes": allocations.bytes,
            "objects": allocations.objects,
            "step": allocations.step,
            "by_type": by_type.iter().map(|(name, amount)| json!({"type": name, "group": memory::group(name), "bytes": amount.bytes, "count": amount.count})).collect::<Vec<_>>(),
            "by_stack": stack_rows.iter().take(500).map(|((stack, ty), amount)| json!({
                "frames": allocations.stacks.stacks[*stack as usize].iter().map(|&(f, l)| json!([f, l])).collect::<Vec<_>>(),
                "type": allocations.types.names[*ty as usize], "bytes": amount.bytes.round(), "count": amount.count.round(),
            })).collect::<Vec<_>>(),
        });
    }

    // heap snapshots and the collector
    if !input.snapshots.is_empty() {
        let last = input.snapshots.last().unwrap();
        let _ = writeln!(
            summary,
            "\nheap at {}: {} live in {} objects (gcinfo() {} KB); the run's own allocations still alive: {}; garbage not collected yet: {}",
            last.label,
            bytes(last.object_bytes as f64),
            group_digits(last.objects),
            last.total_bytes / 1024,
            bytes(last.allocated_since.1),
            bytes(last.unreachable.1 as f64)
        );
        let _ = writeln!(summary, "  by type: {}", last.by_type.iter().take(8).map(|(name, count, size)| format!("{name} {} ({})", bytes(*size as f64), group_digits(*count))).collect::<Vec<_>>().join(", "));
        for held in last.biggest.iter().take(input.config.top.min(8)) {
            let _ = writeln!(summary, "  {:>10} kept alive by {} {}  ({})", bytes(held.retained as f64), held.ty, if held.name.is_empty() { String::new() } else { format!("\"{}\"", held.name) }, held.path);
        }
        if input.snapshots.len() > 1 {
            let _ = writeln!(summary, "  snapshots: {}", input.snapshots.iter().map(|s| format!("{} {}", s.label, bytes(s.object_bytes as f64))).collect::<Vec<_>>().join(" -> "));
        }
        let collector = input.collector;
        let _ = writeln!(
            summary,
            "collector: {} cycles, {} steps, {} freed, {:.1} ms host time",
            collector.cycles,
            group_digits(collector.steps),
            bytes(collector.freed as f64),
            ms(collector.seconds)
        );
        json["heap"] = json!({
            "snapshots": input.snapshots.iter().map(|snapshot| json!({
                "label": snapshot.label, "frame": snapshot.frame, "seconds": snapshot.seconds, "gcinfo_kb": snapshot.total_bytes / 1024, "total_bytes": snapshot.total_bytes,
                "objects": snapshot.objects, "object_bytes": snapshot.object_bytes,
                "by_type": snapshot.by_type.iter().map(|(name, count, size)| json!({"type": name, "count": count, "bytes": size})).collect::<Vec<_>>(),
                "unreachable": {"count": snapshot.unreachable.0, "bytes": snapshot.unreachable.1},
                "allocated_since": {"count": snapshot.allocated_since.0.round(), "bytes": snapshot.allocated_since.1.round()},
                "biggest": snapshot.biggest.iter().map(|held| json!({"type": held.ty, "name": held.name, "size": held.size, "retained": held.retained, "path": held.path,
                    "stack": held.stack.and_then(|stack| input.allocations.map(|allocations| allocations.stacks.stacks[stack as usize].iter().map(|&(f, l)| json!([f, l])).collect::<Vec<_>>()))})).collect::<Vec<_>>(),
                "by_stack": snapshot.by_stack.iter().take(300).map(|(stack, ty, count, size)| json!({
                    "frames": input.allocations.map(|allocations| allocations.stacks.stacks[*stack as usize].iter().map(|&(f, l)| json!([f, l])).collect::<Vec<_>>()),
                    "type": ty, "count": count.round(), "bytes": size.round()})).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
            "gc": {"cycles": collector.cycles, "steps": collector.steps, "freed": collector.freed, "ms": ms(collector.seconds),
                "by_state": memory::GC_STATES.iter().zip(collector.by_state.iter()).map(|(state, (steps, seconds))| json!({"state": state, "steps": steps, "ms": ms(*seconds)})).collect::<Vec<_>>()},
        });
    }

    // ree's caches
    if input.config.kinds.has(super::Kinds::CACHES) {
        let _ = writeln!(summary, "\nree caches");
        let _ = writeln!(summary, "{:<20} {:>8} {:>12} {:>10} {:>10}", "cache", "hit", "lookups", "misses", "evicted");
        for (name, _, counts) in input.caches {
            let lookups = counts.hits + counts.misses;
            if lookups == 0 && counts.evictions == 0 {
                continue;
            }
            let _ = writeln!(summary, "{:<20} {:>8} {:>12} {:>10} {:>10}", name, percent(counts.hits as f64, lookups as f64), group_digits(lookups), group_digits(counts.misses), group_digits(counts.evictions));
        }
        json["ree_caches"] = Json::Array(input.caches.iter().map(|(name, about, counts)| json!({"name": name, "about": about, "hits": counts.hits, "misses": counts.misses, "evictions": counts.evictions})).collect());
    }

    // the timeline
    if !input.rows.is_empty() {
        let rows = input.rows;
        let work: Vec<f64> = rows.iter().map(|row| row.work_ms).collect();
        let (worst, worst_work) = work.iter().enumerate().fold((0, 0.0), |best, (index, &value)| if value > best.1 { (index, value) } else { best });
        let mean = work.iter().sum::<f64>() / work.len() as f64;
        let (first, last) = (&rows[0], rows.last().unwrap());
        let _ = writeln!(
            summary,
            "\ntimeline ({} frames): work a frame mean {:.3} ms, max {:.3} ms (frame {}); host a frame mean {:.2} ms; heap {} -> {}; instances {} -> {}; connections {} -> {}; threads {:+} over the run",
            rows.len(),
            mean,
            worst_work,
            rows[worst].frame,
            rows.iter().map(|row| row.host_ms).sum::<f64>() / rows.len() as f64,
            bytes(first.heap_bytes as f64),
            bytes(last.heap_bytes as f64),
            group_digits(first.instances),
            group_digits(last.instances),
            group_digits(first.connections),
            group_digits(last.connections),
            last.threads - first.threads
        );
        json["timeline"] = json!({
            "columns": ["frame", "seconds", "phase", "host_ms", "work_ms", "instructions", "heap_bytes", "allocated_bytes", "allocations", "gc_steps", "gc_ms", "gc_freed", "instances", "connections", "threads", "cycles"],
            "rows": rows.iter().map(|row| json!([row.frame, round(row.seconds, 4), row.phase, round(row.host_ms, 4), round(row.work_ms, 5), row.instructions, row.heap_bytes, row.allocated_bytes, row.allocations, row.gc_steps, round(row.gc_ms, 4), row.gc_freed, row.instances, row.connections, row.threads, row.cycles])).collect::<Vec<_>>(),
        });
    }

    // the CPU
    if input.config.kinds.has(super::Kinds::CPU) {
        let cpu = input.cpu;
        let _ = writeln!(
            summary,
            "\ncpu: {} cycles on the run thread (reference cycles); process CPU {}; page faults {}; peak working set {}",
            cpu.thread_cycles.map_or("-".into(), group_digits),
            cpu.process_cpu.map_or("-".into(), |(user, kernel)| format!("{:.2} s user, {:.2} s kernel", user, kernel)),
            cpu.page_faults.map_or("-".into(), group_digits),
            cpu.peak_working_set.map_or("-".into(), |peak| bytes(peak as f64))
        );
        if let Some(missing) = &cpu.pmc_missing {
            let _ = writeln!(summary, "hardware counters: not read: {missing}");
        }
        json["cpu"] = json!({
            "thread_cycles": cpu.thread_cycles, "process_cpu_s": cpu.process_cpu.map(|(user, kernel)| json!({"user": user, "kernel": kernel})),
            "page_faults": cpu.page_faults, "peak_working_set": cpu.peak_working_set, "private_bytes": cpu.private_bytes,
            "pmc_sources": cpu.pmc_sources, "pmc": cpu.pmc.as_ref().map(|counters| counters.iter().map(|(name, value)| json!({"counter": name, "value": value})).collect::<Vec<_>>()),
            "pmc_missing": cpu.pmc_missing,
        });
    }

    if let Some(text) = input.phase_timing {
        json["phase_timing"] = json!(text);
        let _ = writeln!(summary, "
{text}");
    }
    if let Some(text) = input.boundary {
        json["boundary"] = json!(text);
        files.push("boundary.json".into());
    }

    // sources of the hottest functions, for the annotated listing
    let mut listed: Vec<i32> = Vec::new();
    for (id, figures) in &function_rows {
        if *id < 0 || names.engine(*id) || listed.len() >= 12 {
            continue;
        }
        if figures.self_work > 0.0 || figures.self_time > 0.0 || figures.self_alloc > 0.0 {
            listed.push(*id);
        }
    }
    let mut listings = serde_json::Map::new();
    for id in &listed {
        if let Some(function) = names.function(*id) {
            if let Some((path, rows)) = listing(input, function, &model, *id) {
                listings.insert(id.to_string(), json!({"path": path, "lines": rows.iter().map(|(line, text)| json!([line, text])).collect::<Vec<_>>()}));
            }
        }
    }
    json["listings"] = Json::Object(listings);
    json["source_hashes"] = Json::Object(input.sources.iter().map(|(name, source)| (name.clone(), json!(format!("{:016x}", fxhash(&source.text))))).collect());

    // folded stacks
    let mut folded_files: Vec<(&str, Vec<(String, u64)>)> = Vec::new();
    if let Some(vm_results) = input.vm.filter(|vm| !vm.nodes.is_empty()) {
        let (work, insns) = fold_tree(vm_results, &names, input.phases);
        folded_files.push(("work.folded", work));
        folded_files.push(("instructions.folded", insns));
    }
    if let Some((stacks, samples, _)) = input.time {
        let mut lines: HashMap<String, u64> = HashMap::new();
        for (&(phase, stack), &ticks) in samples {
            *lines.entry(fold(&names, input.phases, phase, &stacks.stacks[stack as usize])).or_default() += ticks;
        }
        folded_files.push(("time.folded", lines.into_iter().collect()));
    }
    if let Some(allocations) = input.allocations {
        let mut by_bytes: HashMap<String, f64> = HashMap::new();
        let mut by_count: HashMap<String, f64> = HashMap::new();
        for (&(phase, stack, _), amount) in &allocations.by_stack {
            let key = fold(&names, input.phases, phase, &allocations.stacks.stacks[stack as usize]);
            *by_bytes.entry(key.clone()).or_default() += amount.bytes;
            *by_count.entry(key).or_default() += amount.count;
        }
        folded_files.push(("alloc.folded", by_bytes.into_iter().map(|(key, value)| (key, value.round() as u64)).collect()));
        folded_files.push(("alloc.counts.folded", by_count.into_iter().map(|(key, value)| (key, value.round() as u64)).collect()));
        if let Some(last) = input.snapshots.last() {
            let mut live: HashMap<String, u64> = HashMap::new();
            for (stack, _, _, size) in &last.by_stack {
                *live.entry(fold(&names, &[], u32::MAX, &allocations.stacks.stacks[*stack as usize])).or_default() += size.round() as u64;
            }
            folded_files.push(("heap.folded", live.into_iter().collect()));
        }
    }
    for (name, mut rows) in folded_files.clone() {
        rows.retain(|row| row.1 > 0);
        rows.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let mut text = String::with_capacity(rows.len() * 80);
        for (stack, weight) in rows {
            let _ = writeln!(text, "{stack} {weight}");
        }
        write_file(&out.join(name), text.as_bytes())?;
        files.push(name.into());
    }

    // speedscope: one sampled profile per folded file
    let speedscope = speedscope(&folded_files, &input.run.command);
    write_file(&out.join("profile.speedscope.json"), speedscope.to_string().as_bytes())?;
    files.push("profile.speedscope.json".into());

    files.insert(0, "summary.txt".into());
    files.insert(1, "profile.json".into());
    files.insert(2, "report.html".into());
    let _ = writeln!(summary, "\nfiles in {}: {}", out.display(), files.join(", "));
    json["summary"] = json!(summary);
    json["files"] = json!(files);
    let text = serde_json::to_string(&json).map_err(|error| error.to_string())?;
    write_file(&out.join("profile.json"), text.as_bytes())?;
    write_file(&out.join("summary.txt"), summary.as_bytes())?;
    write_file(&out.join("report.html"), html(&text).as_bytes())?;
    Ok(summary)
}

/// A stable key for a function across runs (its module and name, and its line for anonymous ones)
fn function_key(names: &Names, id: i32) -> String {
    match names.function(id) {
        Some(f) if f.name.is_empty() => format!("{}:anonymous@{}", module_name(&f.source), f.line),
        Some(f) => format!("{}:{}", module_name(&f.source), f.name),
        None => names.frame(id),
    }
}

fn fxhash(bytes: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = rustc_hash::FxHasher::default();
    bytes.hash(&mut hasher);
    hasher.finish()
}

/// `phase;outermost;...;innermost @line` (the legacy folded form; the innermost frame carries its line)
fn fold(names: &Names, phases: &[String], phase: u32, stack: &[memory::Frame]) -> String {
    let mut parts: Vec<String> = Vec::with_capacity(stack.len() + 1);
    if let Some(name) = phases.get(phase as usize) {
        parts.push(name.clone());
    }
    for (index, &(function, line)) in stack.iter().rev().enumerate() {
        let frame = names.frame(function);
        if index + 1 == stack.len() && line >= 0 {
            parts.push(format!("{frame} @{line}"));
        } else {
            parts.push(frame);
        }
    }
    parts.join(";")
}

/// The call tree as folded stacks: (path, self work in microseconds) and (path, self instructions)
fn fold_tree(vm: &vm::Results, names: &Names, phases: &[String]) -> (Vec<(String, u64)>, Vec<(String, u64)>) {
    let mut paths: Vec<Option<String>> = vec![None; vm.nodes.len()];
    let mut work = Vec::new();
    let mut insns = Vec::new();
    for (index, node) in vm.nodes.iter().enumerate() {
        let path = if index == 0 {
            phases.first().cloned().unwrap_or_else(|| "startup".into())
        } else if node.function == -1 {
            phases.get(node.callpc.max(0) as usize).cloned().unwrap_or_else(|| format!("phase {}", node.callpc))
        } else {
            let parent = paths[node.parent.max(0) as usize].clone().unwrap_or_default();
            format!("{parent};{}", names.frame(node.function))
        };
        if node.work > 0.0 {
            work.push((path.clone(), (node.work * 1e6).round() as u64));
        }
        if node.insns > 0 {
            insns.push((path.clone(), node.insns));
        }
        paths[index] = Some(path);
    }
    (work, insns)
}

/// speedscope's file format: shared frames, one sampled profile per folded file
fn speedscope(files: &[(&str, Vec<(String, u64)>)], name: &str) -> Json {
    let mut frames: Vec<String> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut profiles = Vec::new();
    for (file, rows) in files {
        let unit = match *file {
            "work.folded" => "microseconds",
            "time.folded" => "milliseconds",
            "alloc.folded" | "heap.folded" => "bytes",
            _ => "none",
        };
        let mut samples = Vec::new();
        let mut weights = Vec::new();
        for (stack, weight) in rows.iter().filter(|row| row.1 > 0) {
            let sample: Vec<usize> = stack
                .split(';')
                .map(|frame| {
                    *index.entry(frame.to_string()).or_insert_with(|| {
                        frames.push(frame.to_string());
                        frames.len() - 1
                    })
                })
                .collect();
            samples.push(sample);
            weights.push(*weight);
        }
        let total: u64 = weights.iter().sum();
        profiles.push(json!({"type": "sampled", "name": file.trim_end_matches(".folded"), "unit": unit, "startValue": 0, "endValue": total, "samples": samples, "weights": weights}));
    }
    json!({
        "$schema": "https://www.speedscope.app/file-format-schema.json",
        "shared": {"frames": frames.iter().map(|frame| json!({"name": frame})).collect::<Vec<_>>()},
        "profiles": profiles,
        "name": format!("ree {name}"),
        "activeProfileIndex": 0,
        "exporter": "ree",
    })
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    std::fs::write(path, bytes).map_err(|error| format!("{}: {error}", path.display()))
}

/// The self-contained report page: the template with the profile's JSON in it
pub fn html(profile_json: &str) -> String {
    const TEMPLATE: &str = include_str!("report.html");
    // (`</` cannot end the script element the data sits in)
    let data = profile_json.replace("</", "<\\/");
    TEMPLATE.replacen("/*PROFILE_DATA*/null", &data, 1)
}
