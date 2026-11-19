# Engine architecture and contract

This document is the technical specification for `roblox-engine-emulator` (`ree`): what each subsystem does, the Roblox Studio rules it enforces, and the file/protocol formats it consumes and produces. Update this document in the same commit whenever an interface or behavioral rule changes.

For installation and builds see [building.md](building.md), for CLI commands see [cli.md](cli.md), and for Rust embedding see [api.md](api.md).


`ree` originated as the Rust rewrite of the Luau emulator inside [roblox-ui-renderer](https://github.com/Anime-Vanguards/roblox-ui-renderer) (`rur`) and was split into its own crate in October 2026. The original Luau-in-Luau emulator spent ~97% of CPU time inside emulator overhead rather than game code; moving the core to Rust with an embedded Luau VM (`mlua`) lets unmodified game Luau interact with native `Instance` userdata just as it does in Roblox. `rur` adds GUI layout, text shaping, input, and scene export on top as a `Layer`.

## Design principles

1. **Parity first.** Everything visible to scripts matches Roblox Studio as measured (citing Studio version/date, fixture, and measurements in code comments), falling back to official Roblox documentation where not yet measured. Unemulated members never silently guess values—they log `Output` entries of kind `missing`.
2. **Speed without behavioral drift.** Hot paths access properties by pre-interned IDs (`NameId`), avoid per-frame allocations across whole scenes, and execute common `Instance` operations in C metamethods without crossing into Rust/mlua closures.

## Module map

```text
src/lib.rs               Engine, Options, State, Hooks, Layer, Extension, compiled-in data
src/emulator.rs          Emulator wrapper ensuring Lua drops before Engine::teardown
src/launch.rs            CLI engine option parsing (Args, accept, parse, prepare) and Host
src/mount.rs             Project mount orchestration (begin_project / Mounting::finish / mount_project)
src/dom.rs               Generational Instance arena, ordered children, sorted or lazy-plan properties,
                         attributes, tags, flags, and invalidation versions
src/reflection.rs        Studio reflection database (data/reflection.json merged over rbx_reflection_database)
src/value.rs             Property Values (rbx-dom Variant + Font, TweenInfo, Content, etc.)
src/signal.rs            RBXScriptSignal / Connection, Changed / GetPropertyChangedSignal, deferred queues
src/scheduler.rs         Virtual clock, task library, RunService phases, recorded Studio clocks
src/cost.rs              Work-clock prices calibrated against Roblox clients
src/instance.rs          Core instance semantics (create, set_parent, raw_set, clone, attributes, tags, fire)
src/derived.rs           Derived property views and paired Changed notifications
src/output.rs            Structured Output log and LogService history
src/services/            Service & class behaviors (TweenService, QueryDescendants, EncodingService,
                         ReflectionService, InputActions, etc.)
src/project.rs, project/ Rojo mounting, models, snapshots, placeholders, asset bundles (model_assets.rs),
                         ModelPlan & lazy instantiation (plan.rs, plan/codec.rs, plan/store.rs), JSON models
src/lua/                 Luau API: raw Instance userdata (raw.rs), datatypes, globals, require, Actors,
                         Luau stubs (services.luau, server.luau, prelude.luau, vector3.luau)
src/world/, src/record/  3D world transforms, spatial queries, joints, gravity, and 3D recorder
src/animation/           AnimationTracks, Animator stepping, KeyframeSequence & CurveAnimation
src/meshes.rs, images.rs EditableMesh and EditableImage
src/path2d.rs            Path2D control points, splines, and curve queries
src/serve/               Warm background service (`ree serve`) and `--service` client
src/watchdog.rs          Script timeout, wall-clock cap, and parent-exit watchdog
src/trace.rs, census.rs  Context.TraceWrites and Context.Census (plus lua/holders.rs for Context.Holders)
src/timing.rs, files.rs  Phase timers, cache file stamps, and long-lived heap (alloc.rs)
build.rs                 Computes REE_CONVERSION_DIGEST keying cached `<bundle>.plan` files
vendor/, patches/        Vendored patched Luau VM, mlua-sys, and Lune datatypes (tools/vendor.mjs)
```

## Verification and determinism

- **`cargo test --release`:** Runs engine unit tests (`src/tests.rs`, scheduler, datatypes, world, recorder, bundles, `Emulator`), patched-VM tests (`tests/luau_vm.rs`), process-guard tests (`tests/process_guards.rs`), standalone release/worker tests (`tests/standalone.rs`, needs Node.js), and host compatibility tests (`tests/host_worker.rs`). Needs no Studio, GPU, game checkout, or network.
- **Vendor check (`node tools/vendor.mjs --check`):** Verifies `vendor/` equals pinned upstream archives + `patches/` ([building.md](building.md#updating-luau)).
- **Live client Luau FFlags:** The VM loads live Roblox client flags from `data/luau-flags.json` (updated via `node tools/luau-flags.mjs`, applied by `patches/luau/14-client-flags.patch` before VM startup). Override with `REE_LUAU_FLAGS=off` (keeps `LuauEmitCallFeedback` and `LuauPromoteProto` enabled under `jit`) or `REE_LUAU_FLAGS=FFlagName=true,FIntName=3`.
- **Deterministic hashing and recordings:** Tables keyed by instances, tables, functions, or threads iterate in deterministic creation order rather than memory-address order (`patches/luau/04-deterministic-hash.patch`, `REE_HASH_SEED`), guaranteeing byte-identical `scene.rbxm`, `playback.json`, `events.json`, `camera.json`, and `summary.json` across runs.
- **Downstream regression gate:** `roblox-ui-renderer` runs fixtures, stories, and Studio comparison replays through `ree`; intentional script-visible changes must be backed by Studio comparisons before updating goldens.

## Layers and extensions

The core engine has no UI layout, text shaping, or input renderer; hosts attach one via `Engine::with_layers(options, layers)`.

During `lua::start` (`Engine.Start`), `ree` installs core behaviors, derived views, 3D world hooks, and static GUI defaults (`services::install_gui_behaviors`), runs each `Layer::install` in order, and then constructs the `DataModel`—keeping instance serials, member dispatch, and frame-phase ordering consistent with or without layers.

- **Registration hooks:** Layers register behaviors (`engine.behaviors`: `add_method`, `add_getter`, `add_setter` via `services::method`/`getter`; lookup walks superclasses and last registration wins), property/hierarchy/instance-freed callbacks (`engine.hooks`), and scheduler phase hooks (`state.sched.add_hook(Phase::Render, ...)`).
- **Per-engine state:** Store host state in an `Extension` (`engine.extension_or_insert_with(T::default)`). `Engine::teardown` invokes every extension's `teardown` so closures and Lua references release cleanly.
- **Core fallbacks without a GUI layer:** `GetMouseLocation` returns `(-1000, -1000)`, `GuiState` stays `Idle`, `TextService:GetTextSize` uses compiled-in Studio advance tables (`src/text_measure.rs`), `UserInputService.MouseDeltaSensitivity` and mouse/gamepad sensitivities default to `1`, and unimplemented GUI members report `missing`.
- **Host bridges into core features:**
  - Input actions: hosts forward device key/mouse transitions via `services::input_actions::deliver`.
  - `Path2D`: hosts supply parent rects via `path2d::set_frame_source` and signal layout via `path2d::laid_out` (without a layout host, `GuiObject` parents measure `0x0` at `(0, 0)` and `ScreenGui`/`SurfaceGui` measure `800x600`).

## Project mount (`src/mount.rs`, `src/project.rs`)

`mount_project(lua, engine, &spec, started)` runs `Engine.Start` (while scanning roots in parallel via `project::Prescan`), then mounts in strict order:
1. Studio snapshots (`<Service>.rbxm`)
2. Rojo roots, or a Rojo project file's tree (`--rojo`)
3. Data roots (`data-roots.json`)
4. Explicit mounts (`--mount`)
5. Package stand-ins (`lua/packages`: UI Labs data shapes)
6. Local player join (`begin_project` pauses here; `Mounting::finish` runs steps 7–8 so hosts like `rur` can inject story setup)
7. Studio hierarchy placeholders (`studio-places.json` / `studio-snapshots/<Place>.json`)
8. Manifest bundles

### Mounting rules

- **Rojo roots:** `<root>/<Service>/...` supports `init.luau`, `*.server.luau`, `*.client.luau`, `init.meta.json`, `.json` modules, and `*.model.json`. A directory containing `default.project.json` (e.g., Wally packages) mounts as that nested project ahead of any init script (using the project's `name`, unless the directory itself was explicitly mounted via `--mount`/`Context.Mount`/`$path`).
  - **Path sandboxing:** Nested `$path` values must remain strictly inside the project file's directory—absolute paths, UNC paths, drive prefixes, and `..` segments are rejected with a warning. Missing/refused root `$path`s or unreadable project files fall back to mounting the raw directory; missing child nodes are skipped (silently for `{ "optional": ... }`). Nesting past depth 32 or 10,000 projects per mount stops with an empty `Folder` and a warning.
  - **Symlinks & junctions:** Followed unless cyclic (a link pointing to an ancestor directory is skipped with a warning).
- **Rojo project files (`--rojo <file.project.json>`, `src/rojo.rs`):** The project's tree becomes mounts in file order (`rojo::Tree`, read by `launch::prepare`), applied where the roots mount. A `"$className": "DataModel"` tree mounts each node's `$path` (relative to the project file; `..` allowed, since the user named the file) at its instance path; a `$path` to another `*.project.json` mounts that project's tree at the node; `$className` nodes without a `$path` below a service become instances of that class (a Folder for an unknown class or a service); `{ "optional": ... }` paths that are missing are skipped quietly, other missing `$path`s are skipped with a warning (`<file> maps A.B to <path>, which doesn't exist; not mounted`, with `wally install` advice for a Wally folder beside a `wally.toml`). Any other tree (a model or plugin project) mounts at `ReplicatedStorage/<project name>` or `--rojo-at Service/Name`. `$properties`, `$attributes`, `$ignoreUnknownInstances` and `globIgnorePaths` are ignored, as roblox-ui-renderer's reader (the same module) ignores them. Files and folders mount exactly as below a root (scripts, `init` files, `.model.json`, `.rbxm`/`.rbxmx`, JSON modules, a folder's own `default.project.json`, StarterPlayerScripts' class). `--rojo` replaces `--roots` (both together are refused); explicit `--mount`s follow it. The project file's folder is the run's `Options::project` unless `--project` names one, and stands for the place's root wherever a root's folder is looked up (`mount::place_roots`: `rur.json` settings, `studio-places.json` place id and hierarchy snapshot with its lazy Studio-only placeholders, data roots, the Studio name map on a server run, manifest bundles, package stand-ins), as a root folder holding the place's `default.project.json` does. The project files and every `$path` checked are noted for prepared runs (`files::note`).
- **Module bytecode reuse:** Multi-engine hosts (`ree serve`) cache module source and bytecode keyed by file size and mtime checked directly on disk at read time (never trusting a stale mount scan).
- **Project settings (`rur.json`):** Keyed by root folder name (first matching root wins): `placeIds`, `modelAssets` (`mount`, `prefixes`), `clientGui`, `streamingTargetRadius`.
- **Studio hierarchy snapshots & place content:** `studio-places.json` maps `<root>/default.project.json` to a `<Place>` name and `studioPlaceId`; `studio-snapshots/<Place>.json` supplies name+class placeholders materialized lazily on lookup/listing so `FindFirstChild` and `WaitForChild` match Studio. Plain directories with no class override inherit their snapshot class (when not Folder/script/service). Non-script files under `<repo>/data/place-content/<Place>/<parent>/*.rbxm` replace matching placeholders with real serialized instances.
- **Data roots & Studio names:** `data-roots.json` mounts matching `<project>/external/<Name>/src` directories as roots. `studio-name-map.json` restores illegal-on-disk Studio names (colons, slashes, trailing dots) on server mounts (client mounts keep filesystem names).
- **Local content files:** Relative local paths (`output/x.mesh`, `file://<path>`) resolve against `Options::project` (`--project`, else the `--rojo` file's folder, else first root's parent, else CWD) for pre-upload testing; recordings emit canonical absolute `file://` URIs.
- **`ModelPlan` & lazy instantiation (`project/plan.rs`):** Models, snapshots, and bundles compile once into `Send + Sync` flat arrays (`ModelPlan`). `instantiate_lazy` creates root instances up front and materializes children/properties on first script access—automatically materializing the full import region before destructive/subtree operations (`plan::before_method`) while keeping serials (`GetDebugId`), child order, refs, and `GetTagged` identical to eager instantiation.
- **Rojo JSON models (`*.model.json`, `project/json_model.rs`):** Matches Rojo 7 (explicit or implicit typed properties resolved via `rbx-dom` and `ree`'s newer reflection database, round-tripped through `rbx_binary`). Invalid files warn and skip. XML and binary models both resolve enums via `ree`'s reflection and preserve unknown properties.

## Asset bundles (`src/project/model_assets.rs`, feature `bundles`)

Projects can declare lazy-loaded model bundles in `<project>/asset-manifest.luau`:

```lua
return {
	["Models/Props"] = { AssetId = 1234567890, Version = 3 },
	["Models/Props/_Loose"] = { AssetId = 2345678901, Version = 1 },
	["Models/Props/Tall Lamp"] = { AliasOf = "Models/Props/Lamp" },
}
```

`rur.json` `modelAssets` (or `--assets-at <Service.Path> --assets <prefix>,...`, `*` for all) mounts only folders and placeholders at startup. Accessing a placeholder imports the bundle in place (replacing the placeholder with the bundle's single root instance while keeping its path leaf name; `_Loose` bundles unpack children directly into the parent folder; multi-root bundles warn and wrap in a folder). `AliasOf` chains lazily clone the target `Model` on explicit name lookup (`FindFirstChild`, `WaitForChild`, indexing—not listings), warning if the target isn't a `Model` or doesn't exist.

### External asset worker protocol (`REE_ASSET_WORKER`)

Hosts can inject a custom `BundleSource` via `engine.set_bundle_source`. By default, `AssetWorker` spawns `REE_ASSET_WORKER` (passing `--offline` when offline) and exchanges newline-delimited JSON:

| Request (stdin) | Reply (stdout) |
|---|---|
| `{"command": "catalog", "project": "<dir>", "manifest": "<file>"}` | `{"bundles": ["Models/Props", ...]}` |
| `{"command": "asset", "uri": "rbxassetid://<id>?version=<v>", "out": "<file>"}` | `{"file": "<path>"}` |

Replies containing `"error": "<text>"` report failure. `ree` never handles worker credentials.

### Standalone path and environment normalization

In standalone release builds (even when `--repo` is passed), clients resolve paths before launching services or moving worker CWD into `<state dir>`:
- Relative `PATH` entries resolve to absolute paths (on Unix, empty entries become the client's CWD; on Windows, empty entries stay empty). `NODE_PATH` ignores empty entries on all platforms.
- **Worker binary resolution:** On Windows, bare `REE_ASSET_WORKER` names resolve once in the client (beside client exe → system/Windows dirs → `PATH`, adding `.exe` if extensionless) so copied services never accidentally run a different binary beside their copy. On Unix, bare names stay bare for OS `PATH` lookup. Paths containing directories resolve against the client's CWD.
- **Normalized service key:** Standalone services receive the full client environment and hash it after resolving known paths (`PATH`, `NODE_PATH`, user/temp dirs, state overrides, `RUR_ENGINE_DUMP`, `RUR_CLOCK`, profile outputs, `REE_ASSET_WORKER`), uppercasing Windows var names, and pinning `RUR_STATE_DIR`. Values are never stored in service records. Unknown relative env vars resolve against `<state dir>` consistently for both direct runs and services.
- **Relative per-user dirs:** Relative values of `LOCALAPPDATA`, `HOME`, `XDG_CACHE_HOME`, `APPDATA`, and `USERPROFILE` are ignored when choosing the default cache path, then absolutized (along with `TMP`, `TEMP`, `TMPDIR`) before reaching workers.
- **Worker output handling:** Standalone workers receive absolute project, manifest, and output paths; relative reply `file` paths resolve in `<state dir>`. If a worker writes a bundle to a different path than requested, `ree` copies it into `<state dir>` so the compiled `.plan` stays cached in state. (Animation replies pointing to the same file via symlink/alias skip copying by file identity.)
- **Caching:** Bundles cache at `<state dir>/model-bundles/asset-<AssetId>-v<Version>.rbxm` with memory-mapped compiled `<file>.plan` files beside them (keyed by `build.rs` conversion digest and file stamp). `recent-*.txt` triggers background pre-decoding on subsequent runs of the same scene.

Checkout builds and embedded library hosts preserve their original relative paths, worker lookup, and service key rules.

## Core engine API and data model (`src/`)

Each mounted project owns one single-threaded (`!Send`) `Rc<Engine>` paired with one `mlua::Lua` (always drop `Lua` before calling `engine.teardown()`; `Emulator` enforces this). Built-in Luau chunks (prelude, services, `vector3.luau`) compile to bytecode once per process (`lua::modules::load_engine_luau`). In loopback mode, the client `Engine` holds a link to its server (`Engine::peer`, `loopback::link`, `loopback::join`) so stepping the client steps both engines together. A host that makes its own client (rur's stories) starts the server with `loopback::start_server`, readies its client with `loopback::ready_client`, links them, and starts the server's harness and the join with `record::run::begin_server`.

- **Fast DOM access:** Read instances via `state.dom.at(id)` and properties by pre-interned `NameId` (`K::Size as NameId`, or `reflection.name_id("X")` cached once at setup), converting with `Value::as_*()`. Call `lua::instance::reached` / `find_child_where` before handing instances to Luau so lazy bundle placeholders materialize first.
- **Writes and invalidation:** `instance::raw_set` stores and queues deferred `Changed`/`GetPropertyChangedSignal` events; `raw_set_silent` stores silently. Both increment `Instance.version`. `hooks.property_changed`, `hooks.property_unchanged`, and `hooks.hierarchy_changed` fire after signal queuing (receive `&mut State`; must not re-borrow `engine.state`). `child_version` tracks child lists and `Dom::structure_version` tracks any reparenting.
- **Derived views (`src/derived.rs`):** Computes linked properties (`Font` ↔ `FontFace`, `Transparency`, `*Content`, `BrickColor` ↔ `Color`, etc.) and emits Roblox's paired `Changed` notifications (`Position`, `AssemblyCenterOfMass`, `Orientation`, `Rotation` on `CFrame` write; `AssemblyMass`/`Mass` on `Size`; `BrickColor` before `Color`).
- **Behaviors & errors:** Register Rust methods/getters/setters on `engine.behaviors` (call `engine.clear_member_cache()` afterward); Luau stubs register via `R.Method` in `lua/services.luau`. Return `Ok(values([...]))`, `Err(Flow::Yield)`, or `Err(Flow::Error(ScriptError::at_caller / plain))`.
- **Lifetime & GC:** Reachable instances preserve userdata identity; unreachable unparented trees collect via `lua::raw::collect` during the render step. Pin Rust-held unparented IDs via `state.dom.pin(id)`/`unpin(id)`. `Content` objects holding an `Object` keep it alive (`tools/parity/probes/content_object.luau`); accessing a freed instance raises a script error rather than panicking.

### Coercion, transforms, and Luau library parity

- **Property coercion (`convert::coerce`, measured in client):** Numbers coerce numeric strings and treat non-numbers as `0`; `int` truncates (clamping overflow/non-finite to min); `int64` rounds half away from zero (`0` on overflow); booleans use Lua truthiness; strings accept strings and numbers. `Vector3` and `CFrame` preserve exact `f32` values. Part `CFrame`, joint `C0`/`C1`, and `BulkMoveTo` keep rotations when `|RᵀR - I| <= 1e-5` per entry, otherwise orthonormalize (`CFrame:Orthonormalize()`, identity for zero column); `Model.WorldPivot` preserves arbitrary rotations.
- **Replicated model precision (`project::received`, verified on 2,347 bundle rotations):** Server load orthonormalizes joint `C0`/`C1`, attachments, and bones (keeping pivots intact), then non-axis-aligned rotations quantize through Roblox's 15-bit smallest-three quaternion wire format (`* sqrt(2)` in `1/16383` steps, orthonormal within ~`1e-4`). Positions, axis-aligned rotations, and `:Clone()` preserve exact floats.
- **Chunk names:** Scripts compile with their full instance path (`ReplicatedStorage.Module:3:`, matching client; `chunknames.luau`). Unlike Roblox, identical sources in distinct modules keep their own names.
- **Luau 0.747 builtins:** `math.isnan`, `math.isinf`, and `math.isfinite` are fast-call constant-folded builtins accepting numbers or numeric strings (`math_classify.luau`). `buffer.readvectorf32x*`/`writevectorf32x*` stay hidden behind `DebugLuauBufferVector*` flags (off in live 0.747; `buffer_vector_f32.luau`). The removed Luau integer type is absent.
- **`CFrame:FuzzyEq(other, eps = 1e-5)` (`cframe_fuzzyeq.luau`, Studio 0.744):** Single precision using `|eps|` (raises if `|eps| > 0.1`; `NaN` returns `false`; accepts numeric strings; exact client arg errors). Position matches per axis if `a == b` or `|a - b| <= |eps| * (|a| + 1)`. Rotation matches if `sqrt(sum((RaᵀRb - I)²)) / sqrt(2) <= |eps|` without FMA.
- **`Path2D` (`src/path2d.rs`, Studio 0.744–0.747, `path2d.luau` — 385/385 lines exact):**
  - `GuiBase` with `Closed`, `Color3`, `Thickness` (`0..100`), `Visible`, `ZIndex`. Reading `Transparency`, `SelectedControlPoint`, `SelectedControlPointData`, or `GetSegmentCount` raises RobloxScript capability errors.
  - Up to 100 `Path2DControlPoint`s (serialized in hidden `PropertiesSerialize`, 49 bytes/point). Out-of-bounds indices raise client errors; mutations fire deferred `ControlPointChanged` even if unchanged. Invalid non-`GuiObject`/non-`LayerCollector` parents raise `Attempting to use Path2D with invalid parent`.
  - Spline rebuilds lazily on curve query or layout (so `SetControlPoints` followed immediately by `Insert`/`Update`/`Remove` past the old spline raises `Spline inserting control point at index I access out of bounds` after mutating the list).
  - Segments use linear, quadratic, or cubic Bezier curves based on non-zero tangents; `GetLength` subdivides to `1e-5 px` flatness; `GetBoundingRect` pads by `(Thickness + 1)/2`; arc-length queries (`GetPositionOnCurveArcLength` / `GetTangentOnCurveArcLength`) reproduce Studio's 32-step `f32` Runge-Kutta walk (including tight-bend drift and `t = 1` wrap quirks).

## Instance userdata API (`src/lua/raw.rs`)

Instances expose 16-byte tagged Luau userdata (`slot`, `generation`, API table pointer) backed by C metamethods (`__index`, `__newindex`, `__namecall` with continuation, `__tostring`, `__type`, locked `__metatable`, `__iter`).

- **Fast C paths:** Atom-cached member dispatch handles plain property reads/writes (per-`(instance, property, version)` datatype cache), `Name`/`Parent`/`ClassName`, child indexing, events, core `Instance` methods (`FindFirstChild*`, `IsA`, `GetChildren`, `GetDescendants`, `FindFirstAncestor*`, `IsDescendantOf`/`IsAncestorOf`, `GetPropertyChangedSignal`, `Get/SetAttribute`, `GetFullName`, `WaitForChild` hits, `HasTag`), `BindableEvent:Fire`, and `Instance.new` without mlua closure overhead.
- **Member resolution & error formatting (`errors.luau`):** Readable scriptable properties → behavior getters/methods/events/callbacks → child by name (so `NotScriptable` properties like `PVInstance.Origin` fall through to children). Engine member errors omit script line prefixes; Luau type/call checks keep them.
- **CamelCase first-letter fallback (`camelcase.luau`):** On **reads** and **method calls** only (never writes), if no exact member or child matches and the key starts with lowercase ASCII, `ree` resolves the capitalized first-letter member (`part.anchored`, `HttpService:jSONEncode`). Dump-defined legacy lowercase aliases (`findFirstChild`, `getService`, `brickColor`, `childAdded`, `game.lighting`) resolve directly first. Datatypes only accept their specific legacy lowercase names (`Vector3.x/magnitude/unit/lerp`, `CFrame.p/lookVector/toEulerAnglesXYZ/...`, `BrickColor.r/g/b` not `R/G/B`, `RBXScriptSignal.connect/wait/connectParallel` not `once`, etc.); enums and libraries require exact case.
- **`Destroy`, signals, and `Bindable*` copying (`signals.luau`, `src/lua/bindable.rs`):** Connected handlers retaining an instance keep it alive until `Disconnect` or `Destroy` (measured Studio 0.740). `Destroy` queues deferred `Destroying` just before removal events (after parent's), running after `Destroy` returns with `Parent` locked. Connection IDs are generational `(slot, generation)`. `BindableEvent:Fire` / `BindableFunction:Invoke` deep-copy tables once on send and again per receiver without metatables (array `1..#t` if `#t > 0`, else string-keyed dict; boolean/thread keys and cycles error; threads arrive as `nil`; buffers copy).

## Scheduler, timing, and guards (`src/scheduler.rs`, `src/watchdog.rs`)

### Frame phase order

**Play mode (`--mode play`, measured in client):**
1. Legacy `wait` / `delay` / `spawn` resumptions → full deferred flush
2. `PreAnimation` → `Animator` step (when enabled)
3. `Stepped` / `PreSimulation` → `Motor6D.Transform` apply → free-flight step
4. `PostSimulation`
5. Due `task.wait` / `task.delay` threads (in scheduled order, using one clock snapshot at phase start) → single deferred pass (newly deferred work waits)
6. `Heartbeat` → `LogService.MessageOut` flush
7. Render step: `BindToRenderStep` callbacks → `RenderStepped` / `PreRender` → `TweenService` step → full deferred flush (tween events first, then callback-deferred work)

Edit mode (`--mode edit`) keeps Studio's edit order (flush after each resumption, tween step right after callbacks; display cadence keeps 240 Hz edit Heartbeats).

### Timer, TweenService, and re-entrancy rules

- **`task.wait` / `task.delay` (`task_delay_time`, Studio 0.746):** Measures elapsed time on `os.clock` (frame time + in-frame script work) and wakes at the first task-queue phase at or past target time. Legacy `wait`/`delay`/`spawn`, `Debris`, and `WaitForChild` warnings use frame time only. Recorded (`RUR_CLOCK`) and frozen clocks add no work time.
- **`TweenService` `Color3` interpolation (`tween_color`, Studio 0.747):** Follows Roblox's live `DFFlagStyleTransitionColorSpace` (off as of 2026-11-13): blends per channel linearly as `a + (b - a) * eased` in `f32` (allowing `Back`/`Elastic` overshoot past `0..1`; `Color3:Lerp` is also linear; `ColorSequence`, `BrickColor`, and attributes are not tweenable). *(Parts store exact `f32` `Color` rather than flooring to `1/255` steps.)*
- **Deferred re-entrancy limit:** Chains of deferred events and `task.defer` stop at depth **80**, logging Roblox's exact `Maximum event re-entrancy depth exceeded...` or `Maximum re-entrancy depth (80) exceeded calling task.defer` messages.
- **Script timeout (`src/watchdog.rs`):** Non-yielding resumptions error with `Script timeout: exhausted allowed execution time` after 10 s work clock (default) or 60 s host time, firing on every safepoint so `pcall` cannot trap it. Watchdog only arms VM interrupts once a resumption crosses 0.1 s. Three timeouts exit `126`; wall-clock cap (`600 + 20 * --seconds` s) exits `124`; parent-process exit triggers `125`.
- **Held-value cap (`src/lua/refs.rs`):** Protects `mlua`'s 1,000,000-slot reference thread by raising a script error at **900,000** slots for `task.spawn`/`defer`/`delay`/`spawn`/`delay`/`Debris:AddItem`/`BindableEvent:Fire` and **950,000** for waits (`task.wait`/`wait`/`Signal:Wait`/`WaitForChild`).

## Work clock (`src/cost.rs`)

`os.clock`, `tick`, `DateTime`, `elapsedTime`, and `GetServerTimeNow` advance within each frame by deterministic, client-calibrated work charges so frame-budgeted scripts behave like they do in Roblox:

- **Luau execution (`patches/luau/02-work-counters.patch`, `15-input-size-work.patch`):** Charges every VM instruction by kind (`INTERPRETED_OPS`, `NATIVE_OPS` for `--!native`), object/table-storage allocations, rehashed keys, C calls, unsafe `GETIMPORT` lookups (`+7.9 ns`, `LUAU_SLOW_IMPORT`), and input-size-scaled library work (`table.find`/`insert`/`remove`/`move`/`unpack`/`pack`/`clear`, string creation/search/case/pattern steps, `utf8`, `buffer` ops, `JSONEncode`/`JSONDecode`). Large new buffers (> 1 KB) and buffer ops (> 64 KB) are counted as `unpriced` because no single per-byte constant fits cache tiers. Engine-internal Luau (`prelude`, `services`, `vector3.luau`) and host C allocations (`lua_sethostcfunction`) are excluded.
- **Engine & datatype operations:** Charges Instance reads (+40 ns for lowercase first-letter fallback), writes, methods, `Instance.new`, `Clone`, DataModel enter/leave, `Destroy`, list returns, child walks, ray/spatial queries, `EditableMesh`, datatype constructors/allocating ops (`datatype_price`), and scheduler resumptions (`RESUMED_WAIT` 0.35 us, `RESUMED_DELAY` 0.20 us, `RESUMED_SIGNAL_WAIT` 0.445 us, `RENDER_STEP_DISPATCH` 1.55 us; first handler on a frame event is free; disconnected deferred handlers keep queue charge only).
- **Cold & first-use modeling (`patches/luau/16-cold-attribution.patch`):**
  - First instance touch/frame: 45 ns (part) / 270 ns (GUI), +320 ns for isolated non-contiguous touches (`COLD_TOUCH_ALONE`).
  - First member use/frame: read +0.5 us, write +1.4 us, call +0.8 us.
  - First engine path/frame (`PATH_FIRST`): Clone 25 us (+class, 55 us total for Part), Destroy 10 us, Workspace enter 13.5 us (26 us w/ Part), PivotTo 16 us, part geometry write 9.5 us, BulkMoveTo 28 us, Emit 4.4 us, QueryDescendants 6.6 us, GetDescendants 3.6 us, emitter Enabled 2.4 us—plus per-instance cold fees (`clone_cold`, `destroy_live`, `PIVOT_COLD_PART`, etc.).
  - First script call/frame: C function +2.5 us, engine-Luau datatype op +0.9 us, rare prototype +0.7 us.
  - First pre-frame Luau object touch/frame (`lua_coldtracking`): table 200 ns, closure 30 ns, proto 5 ns, executed cold instruction 0.85 ns (armed per-instruction via opcode markers so unexecuted branches stay uncharged).
- **GC pricing & deterministic GC clock (`patches/luau/03-gc-clock.patch`):** Charges `8.6 ns` per script-allocated object (`LUAU_COLLECTOR_OBJECT`, booked to the allocating instruction). Luau's GC paces strictly on the deterministic work clock (`lua_setgcclock`) rather than wall time so GC steps fire at identical points across runs.
- **Place & device scaling:** Client `Text` writes with `AutoLocalize` pay ~33 us + 0.43 us/char (`REE_LOCALIZED_PLACES` customizes; `REE_MATCH_PLACES` scales workspace geometry writes by 1.55x). `--device phone` multiplies desktop prices by **2.77x** (calibration range `2.13–4.16x`).
- **Calibration accuracy (`tools/cost-parity`):** Fitted against Studio 0.744 + 0.747 rendered clients (~400 workloads, 1/3 held out): median held-out error **16.5%** (`0.60–1.38x` p10–p90, 175 rows). On 13 held-out macro game workloads (casts, card grid, server loop, scroll, step lists), total clock reads `0.53–1.06x` real client time (`cost::BENCH_BANDS`). `REE_FROZEN_CLOCK=1` freezes in-frame work advances; `Options::clock_probe` (`--timing-check`) scales in-frame work across client variance bands.

## Profiling internals (`src/profiler/`, `src/luau_profile.rs`, `src/boundary.rs`)

- **Instruction & cache profiling (`patches/luau/12-profiling.patch`):** Switches `luau_execute` to a single-step variant booking `lua_workcounters` deltas, engine charges, and cold/collector/dispatch/provenance splits (`lua_profilesplit`) per function, line, phase, and call-tree node (`version: 2`). C calls get their own call-site nodes; protected-call unwinds (`luaP_unwound`) keep nesting accurate without C++ exceptions. Inline cache sites (`GETGLOBAL`/`SETGLOBAL`/`GETTABLEKS`/`SETTABLEKS`/`NAMECALL`/`GETIMPORT`, plus fallback `*UDATAKS`) and `FASTCALL` builtins record per-node/per-phase hits, outcomes, `charged_work_us` (`first_touch_us` + `outcome_charged_us`, `presized_us`), `decline_charged_us` vs. `fast_equivalent_us`, and committed uncharged price ratios (`profiler/prices.rs`). `-O2`-inlined local functions are detected by line runs and counted separately.
- **Memory & heap snapshots:** `onallocate` + `lua_allocblock` + `lua_profilemark` track live allocations per stack; `lua_enumheap` walks all GC roots (including prototype metadata, open upvalues, and weak-table strings) to compute reachable heap, Cooper-Harvey-Kennedy dominators (retained sizes), BFS retainer paths, table array/hash utilization histograms, SCC nesting depths, and `ltable.cpp` `rehash` stacks—keeping uncollected garbage separate without forcing GC.
- **Timeline, issues & boundary:** `timeline.function_stats`, `timeline.issue_stats` (full-run peak/over-budget tracking independent of timeline capping), and over-budget `node_frames`/`site_frames`/`replacement_frames` drive `profile-view` issue pricing (exact vs. estimate vs. at-most vs. part bounds; deduplicated set-union totals; whole-code bounds for source edits except operand-preserving `string.byte(s)` and `string.sub(s, i)`; physical-frame boundary denominators excluding frame 0).
- **Hardware counters (`patches/luau/17-profile-calls.patch`, `src/profiler/hw/`):** Context-switch PMC traces from `ree-hw-counters` split cleanly on helper-thread yield switches (`hw::readings::split`). Adaptive call sampling (`every` tuned between 20 us and 1 ms for ~10k readings/s, 10 us floor / 7.5 us hysteresis, up to 64 recursive calls together, 2M slice cap) produces unbiased per-function counter estimates summing to the run total.
- **Host-time sampling (`src/luau_profile.rs`):** 1 ms ticker checked via atomic load at safepoints, engine steps, and C metamethods, weighted by run-thread CPU cycles (`QueryThreadCycleTime`), tracking `ccalls` per interpreter entry so C-call time isn't mis-attributed to `-O2`-inlined code.
- **Overhead & isolation:** When disabled, hooks cost a single thread-local read (~0.25 ns; `hw` call hooks run in a separate compiled `lvmexecute_profiled.cpp` copy at +0.0% overhead when off). When enabled, profiling allocates zero bytes on the Luau heap and produces byte-identical recordings.
- **Memory figures vs. Roblox:** `gcinfo()` and `collectgarbage("count")` return whole KB including uncollected garbage (baseline includes `ree`'s prelude/services; each Instance userdata is 48 bytes). `Stats:GetMemoryUsageMbForTag` is not emulated.

## 3D world and recorder (`src/world/`, `src/record/`)

- **CFrame views:** `BasePart`/`Attachment` `Position`/`Orientation`/`Rotation` (`YXZ`/`XYZ` in `glam` `f32`) map directly to `CFrame`. `Attachment.World*` resolves relative to the nearest ancestor Attachment, BasePart, Bone (`TransformedWorldCFrame`), or Model (pivot), skipping Folders/Configurations/Accessories (`Workspace`/`WorldModel` = identity).
- **Rigid assemblies (`world/joints.rs`):** `Weld`, `Snap`, `ManualWeld`, `Motor`, `Motor6D` (`Part0 * C0 * Transform = Part1 * C1`), and `WeldConstraint` move rigidly together. `AssemblyRootPart` follows Roblox priority: anchored > non-Massless > `RootPriority` > largest face area (multiplied by `20` Seat/VehicleSeat, `10` `HumanoidRootPart`, `5` `Torso`) > newer creation order. `CFrame` writes move the whole assembly; `Position`/`Orientation`/`Rotation` move one part and update joints. `Model:PivotTo` sets non-`PrimaryPart` pivots directly without compounding float error. `Motor6D.Transform` applies after `PreSimulation` (edit mode: Heartbeat start).
- **`WorldRoot:BulkMoveTo`:** Validates array types, length equality, `BasePart` entries (skipping `nil` slots without shifting indices), and `CFrame` entries before moving anything—matching client error strings and moving destroyed/unparented/out-of-Workspace parts too.
- **Free flight (`world/gravity.rs`, measured in client):** Unanchored, non-colliding, non-Humanoid, unconstrained Workspace assemblies simulate under `Workspace.Gravity` with linear/angular velocity (`ApplyImpulse*`), taking 4 x `1/240 s` substeps on frame 1 and `1/60 s` afterward, box inertia spin decaying 5.2%/s, volume*density mass (`Massless` = 0), and streaming-radius freezing around replication foci (`StreamingTargetRadius` floored to 64 studs, `-32` for added foci). No collisions or `FallenPartsDestroyHeight`.
- **Spatial queries (`world/query.rs`):** `Block` = box, `Ball` = sphere, `Cylinder` = X-axis cylinder, all other parts = OBB; `Terrain` never hits; rays starting inside a part ignore it; respects `CanQuery`, `RespectCanCollide`, `FilterDescendantsInstances`, and `ExcludeInstances`/`IncludeInstances` (no collision groups).
- **Camera projections (`world/camera.rs`):** Perspective math from `CFrame`, `FieldOfView`, and viewport size; `WorldToScreenPoint` / `ScreenPointToRay` offset `Y` by `Options::topbar_inset` (`GuiService:GetGuiInset`).
- **Recorder (`record/mod.rs`):** Captures `Workspace` descendants (skipping scripts, sounds, values, Animators, Humanoids, Terrain, Cameras, Clouds), `CurrentCamera`/`Terrain` children, tracked `Lighting` properties + `Workspace.Terrain.Clouds` (`Color`, `Cover`, `Density`, `Enabled`), Lighting/Camera skies/atmospheres/post effects, `PlayerGui` `SurfaceGui`/`BillboardGui` under adornees, and `PlayerGui` `ScreenGui`s. Rounds values to `1e-5`, simplifies collinear motion keys (`<= 0.002` studs / `<= 2e-4` rot), and suppresses unpopulated Studio-only visual placeholders.
- **`EditableMesh` & `EditableImage` (`src/meshes.rs`, `src/images.rs`):** High-bit typed IDs; face corners inherit newest vertex normal/UV/color or compute `1 - cos²(angle)`-weighted `f32` smooth normals; `BatchAdd` matches client error messages and reverse face numbering (`editable_mesh_batch.luau`); `CreateEditableMeshAsync` decodes meshes via `ror-mesh` (feature `meshes`) including bones/bind poses; bound `MeshPart`s point to state-dir FileMesh snapshots and `Content.fromObject` images point to PNG snapshots. `AssetService:CreateDataModelContentAsync` yields and returns `Success` + an immutable `Opaque` copy (`data_model_content.luau`).

## Animation (`src/animation/`)

Calibrated against Studio 0.747 (`src/animation/curve_golden/`):

- **Loading & latency:** Enabled by `animation::enable` (`--animations` dirs → `<state dir>/animations` → asset worker `asset` command; registered clips always work). First fetch of an asset resolves **0.45 s** later (tracks have `Length = 0` until loaded; `ContentProvider:PreloadAsync` awaits them; subsequent loads are instant). Missing IDs warn once.
- **Blending & posing:** Runs in play mode after `PreAnimation` and before `Stepped`. Priorities (`Action4` → `Core`) each claim `min(summed_weight, remaining)` (negative weights = 0), blending tracks by weight with remainder at rest. While any track steps, every rig `Motor6D`/`Bone` is written each step; when all tracks stop, joints retain their last pose. `Animator:StepAnimations(dt)` works in edit mode and raises a capability error in play mode.
- **Curves (`curve.rs`, `curve_golden/`):** `FloatCurveKey` / `RotationCurveKey` snap to `1/2400 s` ticks (`SetKeys` drops closer duplicates; `RotationCurve:SetKeys` raises `"Variant cast failed"` like Studio 0.747—use `InsertKey`). `FloatCurve:GetValueAtTime` matches Studio bit-for-bit in `f32` (including auto-tangent slope rules). `RotationCurve` interpolates shortest-arc slerp/Hermite within `2e-6 rad`. `EulerRotationCurve` / `Vector3Curve` cache `X`/`Y`/`Z` child channels by name-at-parenting rules (`channels.luau`, `arrivals.luau`). `MarkerCurve` enforces 64-byte marker strings and NUL-termination truncation on serialization (`markers.luau`). Curve data persists in hidden `ValuesAndTimes`.
- **Track stepping (580/580 golden lines exact):** `TimePosition` and `WeightCurrent` are `f32` (`WeightTarget = 0` until `Play`). Steps fire markers in `[start, end)` (keyframe markers before keyframe name). Passing `Length` (or `0` backward) fires end markers + `KeyframeReached("End")` (CurveAnimation only), `Stopped` (if unlooped), and `DidLoop` (whenever overshooting the end, even unlooped); unlooped tracks fade out over **0.3 s**. Negative speed starts at `Length`. `Ended` fires one step after fade reaches `<= 1e-5` (`Stop(0)` leaves `WeightCurrent` and pose intact; weights `< 0.001` don't pose; destroyed tracks keep playing; destroyed `Animator` stops tracks).
- **Pose sampling & packed sequences:** `CurveAnimation` samples at 30 Hz (`k / 30 s` in `f32`) and linearly interpolates between samples, applying Studio's polynomial acos rotation round-trip (translations bit-exact; rotations within `1e-6`). `KeyframeSequence` applies Studio's easing quirks (swapped `Cubic` In/Out, `Constant` In/Out/InOut) and packs evenly spaced all-`Linear` sequences onto a uniform time grid with 15-bit quantized translations (`1/32767` of max axis) and smallest-three quaternions (`sqrt(2)/32766`).
- **`FaceControls`:** Driven from `FaceControls` folders (`FloatCurve`s or `NumberPose`s) onto the rig's first `FaceControls` instance named `"FaceControls"`, blended by priority/weight and clamped to `0..1` (head mesh deformation itself is not simulated).
- **Providers (`rbxtemp://` and `active://`):** Content-addressed `rbxtemp://N` IDs deduplicate identical clips, strip non-`Archivable` descendants, and return one cached instance per ID (`GetKeyframeSequenceAsync` converts `CurveAnimation` at 30 Hz and strips attributes/tags; `GetAnimationClipAsync` preserves them). `active://N` IDs are tracked independently per provider (`LoadAnimation` reads `AnimationClipProvider`'s `active://` IDs and tracks live clip edits).

## Input actions (`src/services/input_actions.rs`)

Matches playing-client behavior (`input_actions.luau`, `input_action_resets.luau`):
- `InputAction:GetState()` returns type defaults (`Bool`: `false`, `Direction1D`: `0`, `Direction2D`: `(0, 0)`, `Direction3D`: `(0, 0, 0)`, `ViewportPosition`: `(-1, -1)`). State changes update immediately and queue deferred `Pressed`/`Released` (for `Bool`) followed by `StateChanged(state)`.
- `InputAction:Fire(state)` requires enabled action and nearest enabled `InputContext` (or DataModel ancestry if no context); `InputBinding:Fire(state)` requires `Type = Scriptable`. Disabling or reparenting resets the action; changing `Type` resets to default.
- Host device input (`services::input_actions::deliver`, called after `UserInputService` events when no `TextBox` is focused) routes to enabled DataModel actions (excluding `StarterGui`/`StarterPack`/`StarterPlayer`) via `Automatic` bindings, prioritized by modified `KeyCode` bindings, summed/clamped/scaled directional composites, and `InputContext.Priority` (newest first on ties, respecting sink contexts).

## Server and edit run contexts (`src/lua/server.luau`, `src/lua/loading.rs`)

- **Server (`--run-context server`, requires `--mode play`, forbids `--player`):** `IsServer`/`IsStudio`/`IsRunning = true`, `IsClient`/`IsRunMode = false`, `IsEdit` forbidden to game scripts (`--studio false` flips `IsStudio`). Empty `Players`, no rendering (`RenderStepped` errors; `PreRender`/`BindToRenderStep` bind silently), tweens step before `Heartbeat`. Includes clean in-memory `DataStoreService` (versions, keys, `OrderedDataStore`), `MemoryStoreService` (sorted/hash maps, queues), `MessagingService`, and `EncodingService` (Zstd + Base64, also available on clients). `FireClient`/`FireAllClients` are no-ops; `InvokeClient` yields until the player leaves (loopback servers reach their paired client; see below).
- **Edit (`--mode edit --run-context edit`):** `IsClient` and `IsServer` both `true`, `EncodingService` enabled, `Players.LocalPlayer = nil` unless `--player` is passed.

### Loopback run context (`--run-context loopback`, `src/loopback`)

Runs a paired server and client (`RunContext::Server` + `RunContext::Client`) on one thread with two `DataModel`s and two Luau VMs. Every frame steps the server first, then the client with identical `dt`. Calibrated against two-process Studio 0.747 loopback captures (`node tools/parity/run.mjs --loopback`, `tools/parity/probes/loopback/`, 3 captures per probe).

- **Startup & join lifecycle:** The server mounts the project and runs `Server` up to its first yield, then the player (`--player`) joins before frame 1 and `Client` starts (client runs `Engine.Start` only, interpreting engine Luau). Use `Context.HoldJoin()` / `Context.Join()` on the server to defer the join to the end of a chosen frame (client clocks start at join); `Context.Leave()` removes the player at the next frame boundary (`PlayerRemoving` on server, client stops stepping, pending server `InvokeClient` calls raise). Both sides read `Context.Side`; server reads `Context.Player` (supports `Context.SetJoinData`). Without `--harness`, each side boots its own enabled place scripts once (server `Legacy` under `Workspace`/`ServerScriptService` and `Server` outside `ServerStorage`; client `StarterPlayerScripts` copied into `PlayerScripts`, `LocalScript`s in `ReplicatedFirst`/player containers, and `Client` scripts outside `ServerStorage`/Starter containers).
- **Initial replication:** The server creates the player (`Backpack`, `PlayerGui`, `StarterGear`), fires `PlayerAdded`, and pairs it with the client's `LocalPlayer` (plus `PlayerScripts`). The client receives a silent copy (no events fired) of children across the 19 replicated services (`Workspace`, `ReplicatedStorage`, `ReplicatedFirst`, `Lighting`, `Players`, `StarterGui`, `StarterPack`, `StarterPlayer`, `Teams`, `SoundService`, `Chat`, `TextChatService`, `LocalizationService`, `MaterialService`, `TestService`, `InsertService`, `MarketplaceService`, `ProximityPromptService`, `PolicyService`)—never `ServerStorage`, `ServerScriptService`, server `Camera`, or `RURFixtures`. Every property replicates except `LocalTransparencyModifier` and `Workspace.CurrentCamera`, along with attributes and tags (unreplicated instance refs become `nil`; server `Script`s replicate without `Source`, while `Client`-context `Script`s, `LocalScript`s, and `ModuleScript`s keep `Source`).
- **Lazy Studio-only instances:** The join copies only what the server has made. Studio-only placeholders from the place's hierarchy snapshot are made on each side by its own scripts' lookups: a copy keeps its original's snapshot node and whether its Studio children exist, and the client makes them when it looks, as a plain client run does. The server's and client's instances of one snapshot node pair when a message first names one (a write, a remote argument, the remote itself), by the instance each was made under and the node (`src/loopback/lazy.rs`); a server instance made this way under a sent one replicates from then on. Lazily imported plans and bundles (which only the server can make) are made on the server at the join and copied. Neither side makes what only the other looked at.
- **Incremental replication & timing:** Post-join server changes in replicated containers (excluding `ReplicatedFirst`) flush at frame end in first-change order: new subtrees arrive whole (`ChildAdded`, no `Changed`), repeated writes coalesce (reverting produces no `Changed`), unparenting keeps client identity with `Parent = nil`, destroys fire `Destroying`/`ChildRemoved`/`AncestryChanged`, physics moves omit `Changed`. Client writes stay local until overwritten by the server. One ordered stream each way delivers at the receiver's next frame start (`--latency <frames>`, default `1`) at the receive point (after legacy `wait()` resumptions, before `PreAnimation`): replication and remote events queue deferred, while `RemoteFunction` callbacks and answers execute immediately at receive.
- **Remotes & queues:** `FireServer`/`FireClient`/`FireAllClients` snapshot args at call time (numbers with subnormals as `0`, byte-exact strings/buffers, datatypes, mapped instances or `nil`, functions/threads/Enums as `nil`, Bindable-style table copies leaving holes for unmapped values, max nesting 299 / 298 for `FireClient` → `Max lua stack size reached`). Reliable events preserve order; `UnreliableRemoteEvent` arrives after reliable events and drops payloads > 1,000 encoded bytes with Roblox's warning (without buffer compression). Unlistened events queue up to **256** messages for the first listener, dropping extras with Roblox's queue-exhausted error logged at powers of 2 (`1, 2, 4, ...`). Calling wrong-side methods or connecting wrong-side events raises client error strings (parentless remotes silently no-op). `RemoteFunction` callbacks run on their own thread, preserve multiple/nil returns, forward errors with script/line (`error(x, 0)` strips location; tables become `""`), support nested/out-of-order calls, and raise on `Context.Leave()`.
- **Output, recording & limits:** Each side keeps its own `RunService`, work clock, script timeout (3rd timeout on either side aborts), and RNG sequence. Output lines merge in chronological order tagged with `"side": "server"|"client"`; `@done` on either side stops the run; `summary.loopback` logs latency, join frame, message counts, and replication stats; `ree record` captures the client world (`--record-side server` captures the server); `ree serve` runs loopback jobs cold.
- **Speed:** The client interprets engine Luau (loaded from the process's single compile); a module or harness both sides load compiles once per run (`lua::modules::share_compiled_modules`: the client loads the bytecode of the server's compile and runs its own copy). Tables cross through Luau's C API (`src/loopback/copier.rs`, same results as the typed copier); a message's table or buffer is kept once and each handler rebuilds its own. A replicated instance arrives as one property vector in name order, as `Clone` copies one. An idle pair frame costs its two engines' frames plus a few compares and allocates nothing of its own (`tests/loopback_alloc.rs`); plain runs register none of the pair's hooks. `cargo test --release --test loopback_bench -- --ignored --nocapture` prints each cost against its target (`tools/bench/loopback/work.luau`).
- **Known differences & omissions (`tools/parity/known-loopback.json`):** `ree` sends exact CFrame rotations (vs. ~`1e-5` wire rounding), `InvokeServer` after `Heartbeat:Wait()` returns 1 frame sooner, and server/client `GetServerTimeNow`/`time()` align at frame start. Not emulated: multiple clients, characters/Humanoids, client network ownership/replication outside remotes, workspace streaming, `ReplicatedFirst` staged loading, packet loss/reordering/throttling, prepared service loopback jobs, and cross-server `MessagingService`.
- **Runtime code loading (Studio 0.747, `loadstring.luau`, `loadmodule.luau`, `core_services.luau`):**
  - `loadstring(src, chunk?)` raises `loadstring() is not available` unless enabled on a server via `ServerScriptService.LoadStringEnabled` or `--load-string`. Truncates at first NUL byte, inherits caller env (`setfenv` overrides per function), returns `nil, err` on syntax error.
  - `debug.loadmodule(module)` requires `--load-module` (`FFlagEnableLoadModule`); compiles `module.Source` into a fresh `function, nil, cleanup` isolated from `require` cache (JSON modules return a fresh decoder closure).
  - `getfenv(0)` and `getfenv` on engine functions walk the stack to return the calling script's environment (`patches/luau/18-script-thread-globals.patch`).
  - `game:GetService` / `FindService` return `nil` for Roblox-internal services (`CoreScriptSyncService`, `CorePackages`, `RobloxReplicatedStorage`, `PluginGuiService`, `RobloxPluginGuiService`, `AppStorageService`, `CSGDictionaryService`), while allowing `CoreGui`, `StudioService`, and `ScriptEditorService` (identity 4 behavior). `ScriptContext` has name `"Script Context"`.

## Output and `LogService`

- `LogService:GetLogHistory()` returns a fresh oldest-first array of `{ message, messageType, timestamp }` (wall-clock Unix seconds), recording `print`, `warn`, errors, and `LogService` calls immediately up to an emulator cap of **10,000** entries (`logservice.luau`).
- `LogService.MessageOut` fires after `Heartbeat` + deferred flush with `(text, messageType, contextCopy)` (messages emitted inside a `MessageOut` handler do not re-trigger `MessageOut`). Script errors include `Stack Begin` / `Stack End` lines.
- `LogService:Output`/`Info`/`Warn`/`Error`/`Log` interpolate `{key}` placeholders (`{{`/`}}` escape), sanitize context copies (functions → `"<function>"`, instances → full name; mixed-key tables stringify non-primitives), and `Error` throws a formatted table with `{ message, template, context, stack }`.

## Known limitations

- `ImageLabel`/`ImageButton.IsLoaded`, `Sound.IsLoaded` (`TimeLength = 1`), and `DataModel:IsLoaded()` immediately return `true` without fetching assets.
- No rigid-body collisions, physics constraints, body movers, particle simulation, or default character `Animate` scripts.
- All Luau HTTP requests are disabled.
- `SecurityCapabilities` are not emulated (`ReflectionService` `Permits` returns a frozen table reporting `missing` on access); `GetPropertiesOfClass`, `GetMethodsOfClass`, and `GetEventsOfClass` are unimplemented.
- Missing member reads on some datatypes (`CFrame`, `Color3`, `UDim2`, `Vector2`, `Ray`, `RaycastParams`, `OverlapParams`) currently return `nil` or slightly different error wording where Roblox raises (`tools/parity/known.json`).
- Primary development platform is Windows (Linux/macOS are built and smoke-tested in CI; off-Windows `ree serve` cannot detect stale dead PIDs via `alive()`, and memory checks use `/proc/meminfo` on Linux only).

## Compiled-in data files

- **`data/reflection.json`:** Public Studio API dump (`version: 0.747.0.7471481`, generated by `tools/dump-reflection.luau`, diffed with `tools/reflection-diff.mjs`), merged over `rbx_reflection_database` (legacy properties retained in `reflection.rs` `PROPERTY_SUPPLEMENT`). Powers `ReflectionService:GetClasses` / `GetClass` (`IsA` / `ExcludeDisplay` filters). Reflects 0.747 quirks (`Spring`/`Bezier` easing rejected by `TweenInfo`/`TweenService`/GUI tweens; GUI tweens require DataModel ancestry; Camera orthographic writes ignored; `Capsule` treated as box; `EditableImage` unsupported combine/alpha errors; `ParticleEmitter.TimeScale` clamped `0..1`; `PluginOrOpenCloud` capability checks on streaming settings).
- **`data/text-advances.json` & `data/fredoka-sizes.json`:** Studio-measured glyph advance/kerning tables (no font files shipped).
- **`data/font-enums.json`:** `Enum.Font` to `rbxasset://fonts/families/*.json` mapping.

## Environment variables and host vs. standalone modes

No environment variable listed here carries credentials; `ree serve` stores only an unkeyed 64-bit hash of the environment (checkout/library hosts hash `RUR_*`, `REE_*`, `ROR_*`, and `LOCALAPPDATA`; standalone binaries hash the entire normalized worker environment).

- **`RUR_*` (shared with `rur`):** `RUR_STATE_DIR`, `RUR_CACHE_DIR`, `RUR_VERBOSE`, `RUR_ENGINE_TIMINGS`, `RUR_ENGINE_DUMP`, `RUR_EPOCH`, `RUR_CLOCK`, `RUR_CAMERA`, `RUR_PLACE_ID`, `RUR_PLAYER`, `RUR_PLAYER_ID`, `RUR_PLAYER_DISPLAY`, `RUR_ROTATION_CLIPPING`, `RUR_PROFILE_CALLBACKS`, `RUR_LUAU_PROFILE`, `RUR_LUAU_PROFILE_CLOCK`, `RUR_LUAU_PROFILE_UNIT`, `RUR_LUAU_ALLOC_PROFILE`, `RUR_LUAU_ALLOC_STEP`, `RUR_BOUNDARY_PROFILE`, `RUR_CHARGE_TALLY`.
- **`REE_*`:** `REE_SCRIPT_TIMEOUT`, `REE_MAX_SCRIPT_TIMEOUTS`, `REE_MAX_WALL_SECONDS`, `REE_PARENT_WATCH`, `REE_SERVE_WORKERS`, `REE_FROZEN_CLOCK`, `REE_TIMING_CHECK`, `REE_RANDOM_SEED`, `REE_WORK_COUNTERS`, `REE_HASH_SEED`, `REE_HASH_ADDRESS`, `REE_LOCALIZED_PLACES`, `REE_MATCH_PLACES`, `REE_ASSET_WORKER`, `REE_PROFILE`, `REE_PROFILE_OUT`, `REE_PROFILE_COUNTERS`, `REE_HW_COUNTERS`, `REE_HW_PIN`, `REE_LUAU_FLAGS`, `REE_VIEW_COLOR`, `REE_VIEW_BLESS`.
- **Runtime/allocator:** `RAYON_NUM_THREADS`, `MIMALLOC_EAGER_COMMIT`, `MIMALLOC_PURGE_DELAY`.

### Host mode vs. standalone CLI mode

- **Library hosts** (never call `init_binary`): `default_repo()` finds the nearest ancestor `Cargo.lock` (or `.`); `state_dir(repo)` resolves `RUR_STATE_DIR` → `RUR_CACHE_DIR` → `<repo>/.cache`.
- **`ree` binary** (calls `init_binary(source, state)` at startup): runs in checkout mode only if an ancestor directory has `Cargo.lock` beside a `Cargo.toml` for package `roblox-engine-emulator`. Otherwise it locks into **standalone mode** for its entire lifetime (`is_standalone` = true): `--repo` is input-only, state resolves `RUR_STATE_DIR` → `RUR_CACHE_DIR` → per-user `ree` cache (`serve --state-dir` takes precedence; fails before writing if no cache dir exists), unnamed outputs write under `<state dir>` (`ree-profile`, `ree-boundary.json`, `what-if/<stem>`, `output/`), and standalone service clients absolutize `--frame-times` and known env paths before dispatch.
