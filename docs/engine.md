# The engine: architecture and contract

This is the contract code builds against: what each part of the engine does, the rules it follows and
the formats it reads and writes. Change it in the same commit when an interface or a rule changes. The
README and its documentation index ([building.md](building.md), [cli.md](cli.md), [api.md](api.md)) cover
building, the command line and the API; this file covers how the engine works inside.


The engine began as the native rewrite of a Luau emulator inside
[roblox-ui-renderer](https://github.com/Anime-Vanguards/roblox-ui-renderer) (rur) and was split out of
it in October 2026. rur still builds its UI previews on it and adds the GUI side (layout, text, input,
scene export) as a layer; rur's own docs are the contract of that side.

## Why

Roblox runs its engine in C++. Running a game's own UI and 3D code offline needs an engine that scripts
cannot tell apart from Roblox's. The first emulator was written in Luau; on its benchmarks the Luau
thread spent about 97% of its time in the emulator and 3% in game code, so the engine became native
(Rust), with the game's Luau still running in a Luau VM (mlua) and seeing Instances as userdata, as in
Roblox.

## Principle: parity first, then speed

Everything a script can see follows Roblox Studio as measured, else Roblox's documentation, and the code
says which. A comment that states a rule cites the measurement (the Studio version or date, the scene,
the numbers) or the documentation page. Nothing is guessed silently: a member that is not emulated
reports itself in the run's Output (kind `missing`) instead of returning a made-up value.

Speed comes second and never changes output: hot paths read properties by interned id, allocate nothing
per frame for whole scenes, and keep the Instance API's common operations in C metamethods ("Instance
API").

## Shape

```
src/lib.rs           Engine, Options, State, Hooks, Layer, Extension; data (the compiled-in dump and fonts)
src/emulator.rs      Emulator: an engine, its Luau VM and its project, dropped in the right order
src/launch.rs        command-line engine options (Args, accept, parse, prepare) and Host
src/mount.rs         the project mount: begin_project / Mounting::finish, mount_project
src/dom.rs           arena of instances: generational InstanceId, class, name, parent, ordered children,
                     properties (their own sorted vector, or a plan node's, read through its lazy import
                     until written), attributes, tags, flags, change versions for invalidation
src/reflection.rs    classes, properties, events, methods, defaults: data/reflection.json (Studio's dump)
                     merged over rbx_reflection_database
src/value.rs         property values: rbx-dom's Variant plus engine-only kinds (Font, TweenInfo, ...)
src/signal.rs        RBXScriptSignal / Connection, Changed / GetPropertyChangedSignal, ChildAdded, ...;
                     deferred signal behaviour as measured
src/scheduler.rs     virtual clock, task library, Heartbeat / RenderStepped / Stepped, frame phases,
                     recorded Studio clocks
src/cost.rs          the work clock's prices: what script work costs in Studio
src/instance.rs derived.rs output.rs lune_json.rs profile.rs microprofile.rs luau_profile.rs
src/services/        per-class behaviours (mod.rs; tween.rs TweenService, query.rs QueryDescendants,
                     encoding.rs EncodingService)
src/project.rs project/   Rojo mounting, models and snapshots, Studio-only placeholders, asset manifest
                     bundles (model_assets.rs), model plans (plan.rs, plan/codec.rs, plan/store.rs), Rojo
                     JSON models (json_model.rs)
src/lua/             the Luau-facing API: Instance userdata (raw.rs), datatypes, globals, require, Actors,
                     Luau stubs (services.luau, server.luau, prelude.luau, vector3.luau)
src/world/ src/record/   the 3D world and the recorder
src/animation/       AnimationTracks: the Animator step, clips, keyframe loading
src/meshes.rs images.rs  EditableMesh and EditableImage
src/serve/           `ree serve` (a warm process running record / run jobs) and its client (`--service`)
src/watchdog.rs      the script timeout, the wall-clock cap, the exit when the parent is gone
src/trace.rs census.rs lua/holders.rs   Context.TraceWrites, Context.Census, Context.Holders
src/timing.rs files.rs alloc.rs   `--profile` timings, file stamps for caches, the long-lived heap
src/main.rs          `ree`
build.rs             the conversion digest (REE_CONVERSION_DIGEST): the key of `<bundle>.plan` files
vendor/ patches/ tools/vendor.mjs   the patched Luau VM and Lune's datatypes
```

## Verification

- `cargo test --release`: the engine's unit tests (script-API tests in src/tests.rs, the scheduler,
  datatypes, the world, the recorder, model bundles, the Emulator) and the patched VM's
  (tests/luau_vm.rs), plus the process guards (tests/process_guards.rs, which run the built `ree`).
  Tests build their own scenes in temporary directories; none needs Roblox Studio, a game checkout, a
  GPU or the network.
- `node tools/vendor.mjs --check`: vendor/ is exactly the pinned archives with patches/ applied.
- Recordings are deterministic: the same harness and project give byte-identical `scene.rbxm`,
  `playback.json`, `events.json`, `camera.json` and summary files. Two runs of one scene make the same
  writes: tables keyed by Instances, tables, functions or threads are visited in an order fixed by the
  objects' creation, not their addresses (patches/luau.patch, `REE_HASH_SEED`).
- Downstream: roblox-ui-renderer runs its fixtures, stories and Studio comparison replays through this
  engine and compares their output with recordings made in Studio. A change to script-visible
  behaviour is checked there too; an intended change of output is re-recorded only after a comparison
  shows it is right in Studio.

## Layers and extensions

The core never calls a GUI: hosts add one. `Engine::with_layers(options, layers)` keeps the host's
`Layer`s; `lua::start` (Engine.Start) installs the core's service behaviours, derived views, world and
static GUI behaviours (`services::install_gui_behaviors`), then each layer's `install` in order, then
builds the DataModel, so instance numbering, behaviour lookup and frame-phase hook order are the same
with or without a layer.

- A layer registers what the core registers: behaviours (`engine.behaviors`: `add_method`,
  `add_getter`, `add_setter`, with `services::method` / `services::getter` wrapping closures; looked up
  through the superclass chain, the last registration of a (class, name) wins), property, hierarchy and
  instance-freed hooks (`engine.hooks`), and frame phases (`state.sched.add_hook(Phase::Render, ..)`).
- Its state is an `Extension` (`engine.extension_or_insert_with(T::default)` returns `&T`; a few types
  per engine). `Engine::teardown` calls each extension's `teardown` where the engine drops its own
  parts, so state that captures the engine (closures, Lua values) is released.
- roblox-ui-renderer's GUI layer is the worked example: TextService's measurements, the layout
  (Absolute* / TextBounds / AbsoluteContentSize / AbsoluteCanvasSize, CanvasPosition clamping, the
  Render-phase pass) and input (UserInputService state, GuiState, scrolling, TextBox editing, drag).
  Without it those members keep the core's static answers (GetMouseLocation (-1000, -1000), GuiState
  Idle), TextService:GetTextSize answers from Studio's measured advances (src/text_measure.rs), and the
  rest is reported as not emulated.

## Project mount (`src/mount.rs`, `src/project.rs`)

`mount_project(lua, engine, &ProjectSpec { roots, snapshots, snapshot_dir, mounts, model_assets },
started)` runs Engine.Start, then mounts, in order: Studio snapshots (`<Service>.rbxm` into their
services), the Rojo roots, the data roots, explicit mounts, package stand-ins (lua/packages: a stand-in
for the UI Labs package's data shapes), the local player joining, the place's Studio hierarchy
(Studio-only placeholders) and the manifest bundles. The roots and data roots are listed on the thread
pool while Engine.Start runs (`project::Prescan`). `begin_project` stops after the player joins and
`Mounting::finish` does the rest, so a host can add its own setup in between (rur loads its story host
there).

- **Rojo roots**: `<root>/<Service>/...` with `init.luau`, `*.server.luau`, `*.client.luau`,
  `init.meta.json`, JSON modules and `*.model.json` models. A root's folder is the project folder that
  the conventions below are read from.
- **Project settings**: `rur.json` at the project folder, keyed by the root folder's name (the first
  root with a value wins): `placeIds`, `modelAssets` (`mount`, `prefixes`), `clientGui`,
  `streamingTargetRadius`. The name is kept from rur, which wrote these files first.
- **Studio hierarchy snapshot**: the project folder's `studio-places.json` lists its places,
  `{"environments": [{"name": "<Place>", "projectFile": "<root>/default.project.json",
  "studioPlaceId": <id>}]}`; a root that is the folder of a place's project file belongs to that place,
  and `studio-snapshots/<Place>.json` (`{"nodes": [{"name", "className", "parent"}]}`, a node's parent an
  index into the list, -1 for the DataModel) is its hierarchy as Studio holds it. Instances the snapshot
  lists that no root or model provides become Studio-only placeholders, made when a script first looks
  up or lists children, so `WaitForChild` and `FindFirstChild` see what Studio would.
- **Place content**: a snapshot lists non-script content by name and class only. Files at
  `<repo>/data/place-content/<Place>/<parent path>/*.rbxm` (Studio's own serialization of those
  instances) replace the placeholders of the same names under that parent with the stored instances,
  and the snapshot adds nothing below them.
- **Data roots**: the project folder's `data-roots.json`, `{"paths": ["<path>", ...]}`, names folders
  that checkouts at `<project>/external/<Name>/src` hold; each one found mounts as a root.
- **Studio names**: the project folder's `studio-name-map.json`, `{"files": {"<root>/<Service>/.../<file>":
  {"studioPathParts": [...]}}}`, gives the Studio names of files whose names a file system cannot hold
  (a colon, a slash, a trailing dot). A server mount renames each mounted instance on such a path whose
  name differs; a client mount keeps the file names.
- **Local files**: content naming a relative local path (`output/x.mesh`, `file://<path>`) is read from
  the project's folder (`Options::project`: `--project`, else the folder holding the first root, else
  the current directory), by the same rules ror uses for what names a file (`Options::local_file`).
  Studio reads no local files; this lets a project use meshes and images before they are uploaded.

Model files (snapshots, models, bundles) convert through `project::ModelPlan` (project/plan.rs): the
structure and property values once, kept as a few flat arrays (nodes, properties, distinct values, the
text of names and strings, attributes, tags, unmodeled values) without shared pointers, so a plan is
`Send + Sync`; `instantiate` makes the instances (creation order, names, properties, attributes, tags,
sources). `instantiate_lazy` makes a plan's roots and the rest as scripts look at it: each made
instance's children are made when a script first finds or lists them, a lazily made instance reads its
properties through the plan until a write gives it its own, and before a script moves or destroys one,
or calls a method that may look below it (`plan::before_method`), the rest of its import region is
made, so events, methods, joints and the recorder see the whole tree. Everything script-visible is as
if the import were made at once: serials (`GetDebugId`), child order, references and `GetTagged`.

Rojo's JSON models (`*.model.json`, project/json_model.rs) mount as Rojo 7 builds them: one instance
named after the file, `className`, `name`, `properties`, `attributes`, `children` (lowercase or Rojo's
older capitalized keys), property values explicit (`{"Vector3": [1, 2, 3]}`) or implicit (resolved by
the property's type in rbx-dom's database), round-tripped through rbx_binary as `rojo build` writes
them. A file Rojo would reject is reported as a warning and not mounted.

## Asset bundles (`src/project/model_assets.rs`, feature `bundles`)

A project can name model assets in an asset manifest: the project folder's `asset-manifest.luau`, a
Luau module returning a table with one bundle a line,

```lua
return {
	["Models/Props"] = { AssetId = 1234567890, Version = 3 },
	["Models/Props/_Loose"] = { AssetId = 2345678901, Version = 1 },
}
```

`rur.json` `modelAssets` (or `ree record --assets-at <Service.Path> --assets <prefix>,...`) chooses
where they mount and which (`*`: every one). Only the folders are mounted at startup, with a
placeholder where each bundle goes. A bundle imports when a script finds, tests or touches its
placeholder or looks inside it: the placeholder becomes the bundle's one instance in place (its class,
properties, attributes, tags and children; its name stays the path's last segment), so a script that
holds it holds the instance. A `_Loose` bundle's members go into its parent folder. A bundle file
without exactly one instance at its top mounts its instances in a folder at its path, with a warning.

Bundles come from a `BundleSource` (`catalog`, `bundle`, `cached`, `shape`), which a host sets with
`engine.set_bundle_source`. The default is `AssetWorker`: a child process (`REE_ASSET_WORKER` names the
executable) that answers one JSON request a line on its stdin with one JSON reply a line on its stdout:

- `{"command": "catalog", "project": "<project folder>", "manifest": "<manifest file>"}` ->
  `{"bundles": ["Models/Props", ...]}`
- `{"command": "asset", "uri": "rbxassetid://<id>?version=<v>", "out": "<file>"}` -> `{"file": "<path>"}`
  (the asset as uploaded, written to `out`)
- a reply with `"error": "<text>"` is a failure; ree passes the worker nothing but the requests and
  `--offline`, and the worker handles any authentication the downloads need itself.

Bundle files are kept in the state dir as `model-bundles/asset-<AssetId>-v<Version>.rbxm`, named by
what they hold, so a file that exists is current and one file serves every project that pins that
version. Each file's converted plan is written beside it (`<file>.plan`, keyed by build.rs's conversion
digest and the file's stamp) and mapped in place where the system allows (Windows), so concurrent runs
share its pages. The bundles a run imported (`recent-*.txt`) are decoded ahead on other threads at the
next mount of the same scene. Tests use their own `BundleSource`, so none needs a worker.

## Engine API (`src`)

One `Rc<Engine>` per mounted project, owned next to its `mlua::Lua` (drop the Lua first, then
`engine.teardown()`; `Emulator` does both). Everything is single-threaded; `Engine` is `!Send`.

```
lib.rs          Engine { reflection: Arc<Reflection>, options: Options, state: RefCell<State>,
                         hooks: RefCell<Hooks>, behaviors, lua: RefCell<LuaRefs>, modules, layers,
                         extensions }
                State { dom: Dom, signals, sched: Scheduler, output, project, collection }
                engine.world() -> World { game, workspace, camera, local_player }
reflection.rs   Reflection (compiled in, shared per process: Reflection::shared()), K (pre-interned names)
value.rs        Value, Str, FontValue, ContentValue; JSON / rbx-dom default decoding
instance.rs     script semantics without Luau calls: create, set_parent (event order), raw_set /
                raw_set_silent / notify_changed, clone, attributes, tags, fire
derived.rs      derived property views (Font of FontFace, Transparency, *Content, ...)
scheduler.rs    Scheduler data and the Luau-running steps (flush_deferred, resume, step, run_until)
```

- **Reading instances**: `let state = engine.state.borrow();` then `state.dom.at(id)` (class, serial,
  name, parent, children, props, attributes, tags, flags, versions). Properties by interned id, never by
  string: `state.dom.raw_get(id, K::Size as NameId)` (the stored value, else the class default), then
  `Value::as_udim2()`, `as_vector2()`, `as_color3()`, `as_number()`, `as_bool()`, `as_enum()`,
  `as_instance()`, `as_str()`. Names every hot path reads are pre-interned in reflection.rs
  `known_names!`; other names go through `reflection.name_id("X")` once at setup. Resolve classes and
  enum items once (`reflection.class_id`, `reflection.enum_item`) and compare ids.
- **Tree**: `dom.children_of(id)`, `dom.parent_of(id)`, `dom.descendants(id)` (GetDescendants order).
  Studio-only placeholders and the children of lazily imported instances exist once materialized
  (`project::materialize`, `project::materialize_lazy_subtree`); engine code that hands a script a child
  it found calls `lua::instance::reached` (or `find_child_where` for class scans) so a bundle's
  placeholder becomes its instance first.
- **Writing computed values**: `instance::raw_set(&mut state, &reflection, &hooks, id, name, value)` fires
  Changed / GetPropertyChangedSignal (deferred); `raw_set_silent` stores without events. Both bump
  `Instance.version`.
- **Invalidation**: `engine.hooks.borrow_mut().property_changed` and `hierarchy_changed` (instance, old
  parent, new parent) run after a change's signals are queued. `Instance.version` changes with every own
  property, attribute or tag change, `child_version` with its child list, `Dom::structure_version` with
  any parenting. A hook receives `&mut State` and must not borrow `engine.state`.
- **Frame phases**: `state.sched.add_hook(Phase::Render | PreRender | Tween | RenderStep | Heartbeat |
  Simulation, ..)`. Hooks run Luau, so they must not hold a `state` borrow across script calls.
- **Behaviours**: `add_method(class, name, Rc::new(|lua, engine, id, args| -> Ret))`, `add_getter`,
  `add_setter` (call `engine.clear_member_cache()` after changing them). Rust functions return
  `Ok(values([..]))`, `Err(Flow::Error(ScriptError::at_caller(msg)))` (the script sees `msg` at its own
  line, like Luau's `error(msg, 2)`), `ScriptError::plain` (no position) or `Err(Flow::Yield)` after
  registering the thread's resumption. Luau stubs register with `R.Method(className, name, function)` in
  lua/services.luau.
- **Values to and from Luau**: `lua::convert::to_lua`, `from_lua`, `coerce` (assignment rules and
  errors); `lua::instance_value(lua, engine, id)` (the one userdata of an instance) and
  `lua::instance_id(&LuaValue)`.
- **Lifetime**: instances a script can reach keep their userdata identity; unreachable parentless
  subtrees are freed by `lua::raw::collect` (run from the render step). Rust code that holds an
  `InstanceId` outside the tree across frames keeps it reachable with `state.dom.pin(id)` / `unpin(id)`,
  queued work (scheduler job and timer arguments are scanned) or an instance property. Caches keyed by
  id drop entries in `hooks.instance_freed` (or check the generation: `dom.get(id)` of a freed id is
  `None`).

## Instance API (raw userdata, `src/lua/raw.rs`)

Instances are tagged Luau userdata (a 16-byte payload: slot index, generation and a pointer to the API
table) whose metatable holds C functions: `__index`, `__newindex`, `__namecall` (with a continuation),
`__tostring`, `__type`, the locked `__metatable` and `__iter`.

- Keys resolve through Luau string atoms: a string maps to its member `NameId` once, and members
  resolve once per (class, name) into a dispatch entry.
- Fast paths without mlua or Luau: plain property reads (primitives, strings, instances, enum items,
  datatypes through a per-(instance, property, version) cache), Name / Parent / ClassName, children by
  name, events, plain property writes, the base Instance methods (FindFirstChild, IsA, GetChildren,
  GetDescendants, FindFirstChildOfClass / WhichIsA, FindFirstAncestor*, IsDescendantOf / IsAncestorOf,
  GetPropertyChangedSignal, Get/SetAttribute, GetFullName, WaitForChild hits, HasTag),
  BindableEvent:Fire and `Instance.new`.
- Everything else calls mlua functions from C. Errors are raised with the calling script line's
  position, after every Rust value is dropped.
- What a key is: a property scripts can read (Studio's dump permits Read; NotScriptable ones such as
  PVInstance.Origin permit nothing), else a behaviour's getter, method, event or callback, otherwise
  the child of that name (so `part.Origin` finds a child named Origin, as in Roblox). Callbacks a game
  script may assign (BindableFunction.OnInvoke, RemoteFunction.OnClientInvoke / OnServerInvoke,
  MarketplaceService.ProcessReceipt, ...) are kept; a read raises Studio's message.
- What holds an instance is what holds it in Roblox (measured in Studio 0.740): a connected handler that
  references the instance keeps it; Destroy (which disconnects the instance and its descendants,
  deferred) or Disconnect releases it; a destroyed tree nothing references is freed whole.
- Signals and connections reuse slots; a connection id is (slot, generation), so a stale
  RBXScriptConnection value or queued handler never reaches the slot's next connection.

## Scheduler API (`src/scheduler.rs`)

`scheduler::step(lua, engine, Some(dt))`, `run_until(lua, engine, t)`, `step_display_duration(d)` /
`step_display(n)` (a display's cadence: edit-mode Heartbeats keep their own 240 Hz deadlines),
`use_recorded_clock(engine, Some(&json))` (`RUR_CLOCK`: a Studio session's recorded Heartbeats, render
dts, clocks and seed), `flush_deferred`, `resume`. `--mode play` runs one Heartbeat and one render step
per frame with Stepped firing; `--mode edit` runs the edit-mode clock.

### Script timeout (`src/watchdog.rs`)

A resumption the scheduler makes (a thread, a handler, a harness; nested ones count with the outermost)
that runs the limit without yielding errors with Roblox's `Script timeout: exhausted allowed execution
time` at its next safepoint in a script's own code, and at every one after until it yields or ends, so a
`pcall` cannot keep it going. The limit is 10 s by default, as Studio's Script Timeout Length (text and
default from Roblox's documentation, not measured), counted in seconds of the work clock, so it fires
under a frozen clock and the same at any machine load; 6 times the limit in host time fires it too
(work the clock misses: natively compiled code). A watchdog thread installs the VM's interrupt only once
a resumption has run a whole 0.1 s tick, so runs whose scripts yield pay one atomic increment per
resumption. Roblox goes on after a timeout; ree stops a run at the third (exit 126). `ree` also caps a
run's wall-clock time (600 s plus 20 s per `--seconds`, exit 124) and exits when the process that started
it is gone (exit 125).

### Held values (`src/lua/refs.rs`)

Every Lua value Rust holds for later (a queued, delayed or waiting thread, a deferred job's argument, a
timer's thread) takes a slot of mlua's reference thread, which Luau caps at 1,000,000. Roblox has no such
count, so a loop that queues work without yielding runs there until its script timeout; here the slots
would run out first. So task.spawn, task.defer, task.delay, spawn, delay, Debris:AddItem and
BindableEvent:Fire raise `<call>: too many threads and values held for scripts (...)` at the caller once
900,000 slots are held, and task.wait, wait, Signal:Wait and WaitForChild at 950,000. The offending script
errors with its traceback, what it queued runs as the scheduler reaches it, and the other scripts go on.
What still fills the stack (one call with a million arguments) errors in the resumed thread instead of
ending the run.

### Work clock (`src/cost.rs`)

`os.clock` (and `tick`, `DateTime`, `elapsedTime`, `GetServerTimeNow`) reads the frame's virtual time
plus the script work charged since the frame began. Every Instance read, write and method call, Instance.new,
Clone (per instance), a subtree entering or leaving the DataModel (per instance), Destroy, the lists
methods return (per element) and EditableMesh's own work are charged what they cost in a Studio
playtest client, as measured with os.clock (cost.rs gives each price with its measurement). A script's
own Luau is charged too, from the VM's work counters (patches/luau.patch `lua_workcounters`: instructions
by opcode, objects and table storage allocated, keys moved as tables grow, C calls and calls into the
engine's own Luau) at costs fitted to Studio on 46 workloads (tools/luau-cost): an instruction 1.6 ns, a
call 8.5 more, an object 22. The engine's own Luau (prelude, services, vector3.luau) stands in for
Studio's C++ and is not charged.

So a loop that slices its work by an os.clock budget per Heartbeat takes as many frames as in Studio, and
a frozen clock (`REE_FROZEN_CLOCK=1`: the clock follows the frames only) makes runs of code that differs
in speed write the same values. The clock never runs backwards and never falls behind the frame's time.
A recorded clock (`RUR_CLOCK`) takes no charges; the busy-loop allowance still ends a loop that only reads
the clock.

Some places cost more than others (`cost::PlacePrices`, `Options::place_prices`):

- a Text write with AutoLocalize on and letters in the text pays a localization lookup in a place whose
  experience has a localization table (about 56-61 us for a one-letter text, 0.43 us a character more;
  an empty place pays nothing);
- a match place's client pays 1.55 times the default price for a part's geometry write in the workspace.

The engine lists no places by default. A command line adds them with `REE_LOCALIZED_PLACES` (`<place
id>[=<us for a one-letter text>],...`, 61.2 us by default) and `REE_MATCH_PLACES` (`<place id>,...`); an
embedding host sets `Options::place_prices`.

## World and recorder (`src/world`, `src/record`)

The 3D side of the engine and the recorder behind `ree record`. They use the dom, reflection, values,
signals, the scheduler, the Lua bindings and the project mount, and nothing of a GUI layer.

```
world/mod.rs     install(engine), terrain, WorldEvent + report, world_cframe, hung_from
world/frame.rs   CFrame arithmetic (glam, f32), Orientation (YXZ) and Rotation (XYZ) Euler forms
world/query.rs   spatial queries: solids, cast_ray, cast_box, overlaps, the WorldRoot methods
world/camera.rs  WorldToViewportPoint / WorldToScreenPoint / ViewportPointToRay / ScreenPointToRay
world/joints.rs  rigid assemblies, the Motor6D.Transform batch
world/gravity.rs free fall
record/mod.rs    Recorder::install / sample / write: what is recorded and how
record/encode.rs values as playback JSON and as rbx-dom variants
record/run.rs    `ree record` / `ree run`: the project, the harness module and Context, the frame loop
record/prepare.rs runs prepared ahead by `ree serve`
```

- **Views**: BasePart and Attachment Position / Orientation / Rotation (and Attachment Axis /
  SecondaryAxis) read and write CFrame. Attachment World* properties read and write the world CFrame of
  what the attachment hangs from (measured in Studio): its nearest ancestor that is an attachment, a part
  or a model, past Folders, Configurations and Accessories; a model's frame is its pivot, a WorldModel's
  or the Workspace's the identity. An attachment under a bone follows the bone's
  TransformedWorldCFrame.
- **Rigid assemblies** (Roblox's documentation, not measured): Weld, Snap, ManualWeld, Motor, Motor6D
  (Part0 * C0 * Transform = Part1 * C1) and WeldConstraint carry the parts they join. Each assembly hangs
  from its root (an anchored part, else by Massless, RootPriority, then the documented size formula). A
  part's CFrame write moves its whole assembly rigidly; Position / Orientation / Rotation move the part
  alone and rewrite its joints. Model:PivotTo and ScaleTo write their parts as one batch.
  Motor6D.Transform applies after PreSimulation (edit mode: at each Heartbeat's start).
- **Free fall** (measured in a Studio playtest client): in play mode an assembly in the Workspace with no
  anchored part, nothing that can collide, no Humanoid and no constraint falls under Workspace.Gravity
  from its first simulation step. A place that streams simulates only near its replication foci (the
  character, Player.ReplicationFocus, Player:AddReplicationFocus parts): an assembly whose bounding sphere
  starts about a focus simulates while its centre distance plus radius stays within StreamingTargetRadius
  cut to a whole 64 studs (32 less for an added focus), then stands still. Not modelled: collisions,
  angular velocity, FallenPartsDestroyHeight.
- **Queries**: Block parts are boxes, Ball parts spheres, Cylinder parts cylinders along X, anything else
  its oriented bounding box (no mesh geometry); Terrain is never hit. Rays starting inside a part do not
  hit it; CanQuery off is skipped, RespectCanCollide skips CanCollide off; collision groups are not
  modelled. FilterDescendantsInstances and the newer ExcludeInstances / IncludeInstances lists follow
  Roblox's documentation (devforum.roblox.com/t/4563575); a part must pass both.
- **World events** (`Hooks::world_events`): `ParticleEmitter:Emit` (with where its emitter was at the
  call), `Clear`, `Trail:Clear`, sound plays and stops. Particles, trails and beams are not simulated:
  a renderer draws them from these events and the recorded switches.
- **What is recorded**: the Workspace's descendants (code, sounds, values, animators, Humanoids, Terrain
  and Cameras skipped with their subtrees), the CurrentCamera's and Terrain's children as roots (Roblox
  draws what a client parents there), Lighting's own properties and the skies, atmospheres and post
  effects a run parents to Lighting or the camera, SurfaceGuis and BillboardGuis in the PlayerGui under
  their adornees, and the PlayerGui's ScreenGuis in the scene's `PlayerGui`. Each instance gets one copy
  in `scene.rbxm` as it appeared, hidden before and after; `playback.json` holds `set` actions, keyed
  tweens for properties that change frame after frame, motion tracks, emits and clears, reparenting and
  references written later; `events.json` holds prints, warnings, errors and sounds with their times.
  Values are rounded to 1e-5, motion keys within 2e-4, and keys a straight line reproduces are dropped.
- **EditableMesh and EditableImage** (src/meshes.rs, src/images.rs, measured in Studio): ids carry their
  kind in the high bits; AddTriangle gives each corner a new automatic normal, a white opaque colour
  and a (0, 0) uv; CreateEditableMeshAsync of a mesh asset (feature `meshes`, decoded by ror's ror-mesh)
  makes one vertex per file vertex with its bones and bind CFrames. A MeshPart bound to an EditableMesh
  points its MeshId at a FileMesh snapshot in the state dir, rewritten when the mesh changes. An image
  assigned through `Content.fromObject` shows as a PNG snapshot of its pixels.
- **Context helpers**: `Context.TraceWrites` names the Luau line behind each Parent or property write
  (src/trace.rs), `Context.Census` the line behind every live thread, connection, timer and instance
  (src/census.rs), `Context.Holders` what keeps a tree outside the DataModel alive (src/lua/holders.rs).

## Animation (`src/animation`)

AnimationTracks run as Roblox's documentation describes them (AnimationTrack, Animator, Motor6D,
KeyframeSequence pages); how a track behaves before its asset arrives is measured in Studio.

- Loading is the host's choice: `animation::enable(&engine, Loading { directories, download })`.
  `ree record` / `ree run` enable it with `--animations` directories (`<id>.rbxm` / `.rbxmx`), the
  download cache (`<state dir>/animations`) and, with feature `bundles`, the asset worker's `asset`
  command. An engine without it loads only RegisterKeyframeSequence's ids. An id that loads nowhere warns
  once per engine and its tracks have Length 0.
- LoadAnimation on an Animator, Humanoid or AnimationController makes an AnimationTrack; Play, Stop,
  AdjustWeight and AdjustSpeed fade and play as documented; KeyframeReached, GetMarkerReachedSignal,
  DidLoop, Stopped and Ended fire in time order.
- The Animator step runs in the play-mode simulation phase right after PreAnimation and before Stepped.
  Per joint, priorities from the highest (Action4 ... Action, Movement, Idle, Core) each take their
  tracks' summed weight of what is left, blended by weight; what is left stays at rest. A Pose moves the
  Motor6D whose Part0 / Part1 names are its parent Pose's and its own, else the Bone of its name.
- Fetching (measured in Studio playtests): an asset's first request in a session arrives 0.45 s later;
  tracks and preloads of it share the fetch, and one fetched before loads at once. Until it arrives a
  track has Length 0 and never poses. ContentProvider:PreloadAsync waits for the Animations it is given.
- Not emulated: CurveAnimations, the default `Animate` script, animation graphs, RootMotion.

## Server run context (`--run-context server`, `src/lua/server.luau`)

A server is a Studio Play-mode server as Roblox's RunService documentation tabulates it: `IsServer`,
`IsStudio` and `IsRunning` true; `IsClient`, `IsRunMode` and `IsEdit` false. It needs `--mode play` and
refuses `--player`.

- No local player: Players starts empty and a harness adds players with `Instance.new("Player")`.
  Nothing renders: BindToRenderStep, RenderStepped and PreRender never run.
- In-memory DataStoreService (GetAsync, SetAsync, UpdateAsync, IncrementAsync, RemoveAsync, versions,
  keys, OrderedDataStore pages), MemoryStoreService (sorted maps, hash maps, queues) and MessagingService
  (this server's own subscribers), all empty at the start of every run. Limits and error codes follow
  Roblox's documentation where it gives them.
- Remotes: FireClient / FireAllClients reach no client; InvokeClient waits until its player leaves.
- EncodingService (Zstd compression and Base64) in a server; a client keeps it unemulated.
- LogService.MessageOut and ScriptContext.Error fire for a server's own output.
- `--studio false` makes `RunService:IsStudio()` false, as on a live server, so a game takes its live
  branches.

A print of exactly `@done` ends `ree run` / `ree record` at the end of that frame.

## Known limitations

- ImageLabel and ImageButton report `IsLoaded` true at once; Sounds report `IsLoaded` true and a
  `TimeLength` of 1; `DataModel:IsLoaded()` is true. No asset is fetched for them.
- Physics beyond free fall (collisions, constraints, body movers), particles and the default character
  scripts are not simulated.
- Script HTTP requests are disabled: no script reaches the network.
- The engine is developed and tested on Windows. Other systems build and run, but some fallbacks are
  weak: off Windows a `ree serve` record of a dead process is not detected (`alive()` always answers
  true), and memory is read from `/proc/meminfo` (Linux only).

## Data files

- `data/reflection.json`: Studio's API dump (ReflectionService: classes, enums, members, permissions,
  serialization) plus the Instance.new defaults of every creatable class, written by
  tools/dump-reflection.luau in Studio and compiled in. It is merged over rbx_reflection_database. It is
  Roblox's public API metadata, as Roblox publishes in its own API dump; it holds no engine code.
- `data/text-advances.json`: per-glyph advance fits for each Enum.Font face, measured in Studio (no font
  files are included).
- `data/fredoka-sizes.json`: FredokaOne advances and kerning at integer sizes 4-100, from Studio.
- `data/font-enums.json`: the Enum.Font to `rbxasset://fonts/families/*.json` mapping.

## Environment

None of these carry credentials; `ree serve` keys a service by an unkeyed 64-bit hash of the `RUR_*`,
`REE_*` and `ROR_*` variables (and `LOCALAPPDATA`), never their values. The `RUR_*` names are kept from
rur, whose tools set them.

- `RUR_STATE_DIR` / `RUR_CACHE_DIR` (caches; default `<repo>/.cache`), `RUR_VERBOSE`,
  `RUR_ENGINE_TIMINGS`, `RUR_ENGINE_DUMP`, `RUR_EPOCH`, `RUR_CLOCK`, `RUR_CAMERA`, `RUR_PLACE_ID`,
  `RUR_PLAYER`, `RUR_PLAYER_ID`, `RUR_PLAYER_DISPLAY`, `RUR_ROTATION_CLIPPING`, `RUR_PROFILE_CALLBACKS`,
  `RUR_LUAU_PROFILE`, `RUR_LUAU_PROFILE_CLOCK`, `RUR_LUAU_PROFILE_UNIT`, `RUR_LUAU_ALLOC_PROFILE`,
  `RUR_LUAU_ALLOC_STEP`, `RUR_BOUNDARY_PROFILE`, `RUR_CHARGE_TALLY`.
- `REE_SCRIPT_TIMEOUT`, `REE_MAX_SCRIPT_TIMEOUTS`, `REE_MAX_WALL_SECONDS`, `REE_PARENT_WATCH`,
  `REE_SERVE_WORKERS`, `REE_FROZEN_CLOCK`, `REE_RANDOM_SEED`, `REE_WORK_COUNTERS`, `REE_HASH_SEED`,
  `REE_HASH_ADDRESS`, `REE_LOCALIZED_PLACES`, `REE_MATCH_PLACES`, `REE_ASSET_WORKER`.
- Others: `RAYON_NUM_THREADS`, `MIMALLOC_EAGER_COMMIT`, `MIMALLOC_PURGE_DELAY`.
