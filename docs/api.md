# Rust API (`roblox_engine_emulator`)

Guide to embedding `roblox_engine_emulator` in a Rust host, extending it with custom layers (such as a GUI engine), and depending on the crate from Cargo. For the underlying behavioral contract, see [engine.md](engine.md).

## Minimal example

```rust
use roblox_engine_emulator::{Emulator, Engine, Options, mount::ProjectSpec};

let engine = Engine::new(Options { mode: "play".into(), repo: my_checkout, ..Options::default() })?;
let emulator = Emulator::start(engine, &ProjectSpec { roots: vec![shared_root, game_root], ..Default::default() })?;
let call = emulator.run("Probe", "print(workspace.CurrentCamera.FieldOfView) task.wait(1)")?;
emulator.run_until(2.0)?; // or emulator.step(1.0 / 60.0) frame by frame
for line in emulator.output() {
    println!("{} {}", line.kind, line.message);
}
```

## Core building blocks

- **`Engine` (`src/lib.rs`):** Single-threaded (`!Send`), one per mounted project. Construct with `Engine::new(options)` or `Engine::with_layers(options, layers)`. Exposes `state` (DOM, signals, scheduler, output, project state), `reflection`, `behaviors`, and `hooks`. Call `engine.teardown()` after dropping the Lua VM to break reference cycles (`Emulator` manages both automatically). `Options` configures viewport, pixel scale, mode, place, local player, `repo` (host checkout for state caches), and work-clock device pricing.
- **Mounting (`src/mount.rs`):** `mount_project(lua, engine, &spec, started)` runs `lua::start` plus snapshots, Rojo roots, data roots, explicit mounts, package stand-ins, local player join, Studio hierarchy, and manifest bundles. Use `begin_project` + `Mounting::finish` to inject custom setup right before player join. `launch::parse`, `Args::accept`, and `prepare` translate CLI engine options into `Options`, roots, and mounts. `ProjectSpec::rojo` (a `rojo::Tree`, read by `prepare` from `--rojo`) mounts a Rojo project file's tree in the roots' place.
- **Rojo project files (`src/rojo.rs`):** `rojo::mounts(file, at, &mut warnings)` reads a `*.project.json` into its mounts (`--mount` / `--instance` pairs in file order), `rojo::mappings` lists every `$path` with whether it exists, and `project_file_in`, `missing_path`, `wally_advice` and `several_projects` help a host find and explain projects; `rojo::Tree::read` gives the engine's mounts for a run.
- **Executing Luau (`src/lua/`):**
  - `lua::globals::call(lua, engine, function, args)` runs a Luau function on a scheduler thread (may yield; inspect `Call::outcome`).
  - `lua::modules::environment` creates script environments; `lua::modules::require` loads a `ModuleScript`; `replace_source` swaps module source code.
  - `lua::instance_value` / `lua::instance_id` convert between Rust instance IDs and Luau values.
- **Virtual clock & scheduler (`src/scheduler.rs`):** `scheduler::step(lua, engine, Some(dt))`, `run_until(lua, engine, t)`, `step_display_duration` (display refresh cadence), `flush_deferred`, and `use_recorded_clock` (replaying a recorded Studio clock).
- **3D world (`src/world/`):** World CFrame views for parts and attachments, spatial queries (`world::query`: `solids`, `cast_ray`, `cast_box`, and `WorldRoot` methods), `Camera` projections, and `Hooks::world_events` (particle emits, clears, sound plays via `world::WorldEvent`).
- **3D recording (`src/record/`):** `Recorder::install(engine, fps)` → `recorder.sample(engine, frame)` after each frame → `recorder.write(engine, out, extra)`. High-level CLI driver: `record::run::record(&args, &host)` (or `record::run::execute` + `record::run::release` to retain/defer freeing the emulator).
- **Layers and extensions:** Implement `Layer::install(&self, engine: &Rc<Engine>)`, which executes during `Engine.Start` after core behaviors/world setup and before `DataModel` construction. Layers register class behaviors (`engine.behaviors.borrow_mut().add_method` / `add_getter` / `add_setter`), property/hierarchy hooks (`engine.hooks`), and scheduler phase hooks (`state.sched.add_hook(Phase::Render, ...)`). Store per-engine state in an `Extension` (`engine.extension_or_insert_with(...)`, `engine.extension::<T>()`) whose `teardown` releases back-references to the engine. See `roblox-ui-renderer`'s `native/src/engine/mod.rs` for a full GUI layer.
- **Bundles (`src/project/model_assets.rs`):** Call `engine.set_bundle_source(Box<dyn BundleSource>)` to replace the default `AssetWorker`. `cached` reports pre-cached files for eager decoding; `shape` chooses between `BundleShape::Uploaded` (single root replaces placeholder) and the default `BundleShape::Members` (root's children mounted in a folder at the bundle path).
- **Reusable model plans (`src/project/`):** `ModelPlan::new(&reflection, &dom, roots)` compiles a decoded model into compact `Send + Sync` flat arrays. Instantiate eagerly with `plan.instantiate(engine, parent)`, or lazily on script access with `plan.instantiate_lazy`, `instantiate_lazy_as`, or `instantiate_lazy_members` ([engine.md](engine.md)). Multi-engine hosts can share compiled bytecode (`lua::modules::reuse_compiled_modules`) and decoded bundles (`project::model_assets::keep_bundles`).
- **Profiling (`src/profiler/`):** Build a `profiler::Config` (`from_args` or `new`) and call `profiler::Session::begin(config)` before engine startup, hook the VM with `luau_profile::install(&lua)`, mark phases with `luau_profile::set_phase`, collect before VM drop via `profiler::collect(&lua)`, and finish with `session.finish(Run { ... })`. Hosts can register custom caches via `profiler::caches::named` + `named_hit` / `named_miss` / `named_evicted`.
- **Reusing `ree` CLI commands:** Supply a `launch::Host` (name, checkout, engine factory) to `record::run::cli(argv, &host, true)`—this is how `rur-native record` invokes `ree`'s recorder with `rur`'s GUI layer.

## Class, member and enum metadata

`reflection::Reflection` is Studio's API as the engine knows it, readable on its own (no `Engine` needed) by an editor, a documentation generator or a linter. `Reflection::shared()` gives the compiled-in database (`data/reflection.json`, Studio 0.747's ReflectionService, merged over rbx-dom's database), loaded once per process.

```rust
use roblox_engine_emulator::reflection::{Reflection, ReflectedMember};

let reflection = Reflection::shared()?;
let part = reflection.class_id("Part").unwrap();
for member in reflection.members(part) {
    let name = reflection.name(member.name());
    match member {
        ReflectedMember::Property(property) => println!("{name}: {} [{}]", property.script_type_name, property.category().unwrap_or("")),
        ReflectedMember::Method(method) => {
            let parameters: Vec<String> = method.parameters.iter().map(|p| format!("{}: {}", p.name, p.type_info.script)).collect();
            println!("{name}({})", parameters.join(", "));
        }
        other => println!("{name} ({:?})", other.kind()),
    }
    if let Some(message) = member.deprecation_message() {
        println!("  deprecated: {message}");
    }
}
```

- **Classes and enums:** `all_classes()` (the dump's classes in its order, then those only rbx-dom describes; `ClassInfo::reflected` tells them apart), `class_id` / `class_by_name`, `class_chain(class)` (root first), `all_enums()`, `enum_by_name`, `enum_items_of(enum)` (`GetEnumItems` order) and `legacy_names(item)` (`Enum.Font.Montserrat` reads as Gotham). `ClassInfo` has `category()`, `deprecation_message()`, `permits` (`New`, `GetService`), `creatable`, `service`, `superclass` and `subclasses`.
- **Members:** `members(class)` lists every member, own and inherited, in a stable order: class by class from the root of the chain (Object, Instance, ...) to the class itself, each class's properties, methods, events and callbacks in the dump's order; a member a class declares again (a value object's `Changed`) is listed once, where that class lists it. `own_members(class)`, `own_properties`, `own_methods`, `own_events` and `own_callbacks` give one class's own. `member(class, name)` / `member_by_name("Part", "Anchored")` find a member of any kind, own or inherited: a property, else a method, an event or a callback. Unlike an instance's member read, they also find a property scripts cannot read (`PVInstance.Origin`, which a script's `part.Origin` passes over for a child of that name; `PropertyInfo::scriptable` says which), and a deprecated lower-case member as itself (`brickColor`; `property_alias` and `method_alias` give the member a script's read takes it for). `class_property` finds the reflection's properties only, as the engine reads them; `class_method`, `class_event` and `class_callback` find one of a kind. The lookups and lists include the members a class has only through the engine's behaviours: Player's methods (`Kick`, `GetMouse`, `LoadCharacter`, ...), scripts' `Source` and `TextBox.LocalizedText`. A `ReflectedMember` is one of `PropertyInfo`, `MethodInfo`, `EventInfo` and `CallbackInfo`, with `kind()`, `name()`, `owner()` (the declaring class), `category()`, `deprecation_message()`, `permits()`, `parameters()` and `origin()`.
- **Properties (`PropertyInfo`):** the script type (`script_type`, `script_type_name`), the engine type, the enum (`enum_name`, `enum_type`), the class an Instance property names (`instance_type`: `Model.PrimaryPart` is a `BasePart`), `read_only`, `scriptable`, `serialized`, `display` (Studio's `Display`: `category()` is the group the Properties panel files it under, `deprecation_message()` is set when it is deprecated) and `permits`. `default_value(class, name)` is the value a new instance reads.
- **Methods, events and callbacks:** `parameters` (each a `ParameterInfo`: `name`, `type_info` and `default`, the encoded `DefaultValue`, with `default_text()` for a signature and `Reflection::parameter_default` for an engine value), `returns` (a `TypeInfo`: the script and engine type, the enum, the class an Instance type names), `permits` and `display`. The engine reads none of this: it is built from the dump on the first call that asks (some milliseconds) and kept for the process.
- **Permits:** what a game script may do and which security capabilities a sandboxed script needs for it (ReflectionService's `Permits`): `get("Call")` is the capabilities text (`""` for none, `"Players"`, `"Basic | UI"`) or `None` where a game script may not, `allows`, `capabilities` (the names), `iter`. A method has `CallParallel` when a parallel (desynchronized) thread may call it, as Roblox's API reference marks it thread safe (`MethodInfo::callable_in_parallel`); `ReadParallel` is on 38 controller properties only, so a property's absence of it says nothing.
- **Where a member comes from (`MemberOrigin`):** `Dump` (Studio's ReflectionService: every field above), `RbxDom` (rbx-dom's database, for the classes the dump does not list, such as `Player`: types but no display data or permits) or `Engine` (a member the engine has that neither lists, such as `BindableFunction.OnInvoke`, Player's methods and scripts' `Source`: a property's type is known, but ree records no parameters or return type for a method, event or callback of this origin (`parameters` is empty and `returns` is `None`), and no permits or display data).
- **rbx-dom's view:** `dom_class_tags(class)` (`ClassTags`: `not_browsable`, `not_replicated`, `settings`, ...), `dom_property(&property)` (`DomProperty`: `PropertyTags` such as `hidden` and `not_replicated`, `Scriptability`, and `Serialization`: how place and model files store it, `SerializesAs("size")` for `BasePart.Size`) and `dom_properties(class)` (every property rbx-dom lists for a class, file-only ones included). rbx-dom's database is an older Roblox release's (`dom_version()`), so a newer member has none.
- **Not in the data:** which parameters are optional (only those with a `DefaultValue` say so), which methods yield, whether a property is safe to read from a parallel thread, callbacks' signatures (Studio's ReflectionService lists no callbacks), and enum or enum item deprecation.

The public structs are `#[non_exhaustive]`: read their fields, but get them from the database rather than building them, so fields can be added without breaking your code.

## Depending on the crate from Cargo

```toml
[dependencies]
roblox-engine-emulator = { git = "https://github.com/Anime-Vanguards/roblox-engine-emulator", rev = "<commit>" }

# Cargo only applies [patch.crates-io] from the workspace root, so dependents must repeat
# the patched Luau VM and mlua-sys pins at the exact same commit:
[patch.crates-io]
luau0-src = { git = "https://github.com/Anime-Vanguards/roblox-engine-emulator", rev = "<the same commit>" }
mlua-sys = { git = "https://github.com/Anime-Vanguards/roblox-engine-emulator", rev = "<the same commit>" }
```

Pin exact versions of shared crates (`mlua`, `rbx-dom`, `serde_json`) to match `Cargo.toml` here.
When developing against a local checkout of both repos, add a path patch in an untracked `.cargo/config.toml`:
`[patch."https://github.com/Anime-Vanguards/roblox-engine-emulator"] roblox-engine-emulator = { path = "..." }`.
