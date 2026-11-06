# Command line

`ree record`, `ree run` and `ree serve`: harnesses, options, outputs, the warm service and the environment.
The [README](../README.md) has a first run; [engine.md](engine.md) is the contract behind it.

## Commands

```sh
ree record --harness path/to/Harness.luau --out out/scene --roots "game/shared;game/src" \
    --seconds 3 --fps 60 --context '{"Scenario":"demo"}' --assets-at ReplicatedStorage.Bundles --assets Models \
    [--animations <dir>]                             # pre-fetched KeyframeSequences, <id>.rbxm
ror video out/scene/scene.rbxm --playback out/scene/playback.json --duration 3
ree run --harness path/to/Probe.luau --seconds 1     # no recording: the status and every output line as JSON
ree run --harness path/to/Server.luau --run-context server --seconds 60   # a Play-mode server; `@done` ends it early
ree record --service ...                             # the same, run by the warm service (started when missing)
ree record ... --profile                             # where the time went, on stderr
ree run ... --profile boundary [--profile-out f.json]  # Luau-to-engine crossings per call site, priced (src/boundary.rs)
ree serve --status                                   # the running services; --stop asks them to exit
ree serve --warm --roots "game/shared;game/src" --assets-at ReplicatedStorage.Bundles --assets Models
                                                     # start the service and read in what a first recording reads
```

## Harnesses

- The harness is a ModuleScript (mounted as `ReplicatedStorage.RURFixtures.<name>`) returning
  `function(Context)` or `{ Run = function(Context) }`. Its own modules are its children: a
  directory named after it beside the file (`Harness.luau` and `Harness/`), or the folder of a
  `Harness/init.luau`, mounts below it as a project directory does, so `require(script.Server)`
  works. It requires the game's modules through
  services, builds what a server would replicate (tagged parts with attributes, a fight folder, a
  floor), connects `RunService` as the game's loops do, and may yield. `Context` has `Seconds`, `Fps`,
  the `--context` object's fields and `StandIn(Module, Source)`, which gives a ModuleScript another
  source before anything requires it (a stand-in for a module the run cannot have; a harness should
  say why in a comment). `Context.Mount(Path, File)` mounts a model file or directory (an absolute
  path) at an instance path as `--mount Path=File` does, from the harness, and returns the instance
  there. `Context.TraceWrites(Properties?, Depth?, Root?)` returns `Drain()`, the Luau call sites
  (`Chunk:line`, innermost `Depth` frames) of every Parent write and every write of the listed
  properties since its last call (a listed `"*"` watches every property and SetAttribute, Kind
  `"attribute"`), for censuses that must name the script behind a Deferred event
  (docs/engine.md). `Context.Census(Depth?)` returns `Take(Collect?)`, a JSON census of what a run
  holds now by the Luau line behind it: live threads by the line that made them (with where each is
  suspended and what will resume it), connections by the line that connected them, handler runs and
  threads made since the last Take, pending task.wait / task.delay / deferred resumptions, and live
  instances by class and top-level parent; printed every N virtual seconds of a long run, it shows
  what accumulates (src/census.rs). `Context.Holders(Filters, Examples?, Requeue?)` names what keeps
  the trees outside the DataModel whose root starts with a filter alive: a property of a live
  instance, queued work, a pin, or the shortest Luau way from a module result, a handler, a thread,
  the globals or the registry (src/lua/holders.rs). `Context.Checkpoint()` marks where a run's setup ends (the game's modules
  required, the world built: what every run with the same arguments does alike) and what is read
  afresh each run begins (the module being worked on); it does nothing to the run, and a service
  prepares the next run up to it (`--service` below). Only its first call, on the harness's own
  thread before the harness first yields, counts. `Context.FrameCheckpoint()` is the same for a setup
  that runs frames (a match server's boot, in a `Prepare`): called on the harness's own thread after
  it first yielded, the harness waits there and the run resumes it before the next frame, in every
  run alike; a service prepares the next `ree run` up to there for the jobs with the same `--fps`
  (and `--frame-times`). `Context.ReseedRandoms(Seed)` restarts every Random alive (one a setup made
  before the run's seed was known included) from Seed and the order it was made in, and returns how
  many it restarted. `Context.SetRandomSeed(Seed)` reseeds math.random and the next unseeded
  Random.new() from then on. `Context.WorkSeconds()` is the script work charged so far in seconds of
  Studio's time, the same however fast the host is. A server run's `Context.SetJoinData(Player,
  JoinData)` gives a fake player, before it joins, the join data a teleport would have carried
  (`TeleportData`, `Members`, ...), which `Player:GetJoinData()` then returns as a fresh copy.
- A harness may split its setup instead: `return { Prepare = function(Context) ... end, Run =
  function(Context) ... end }`. `Prepare` gets a Context with `StandIn`, `Mount` and `Checkpoint` but
  without the `--context` fields, `Seconds` and `Fps`, so it can only do what every run of the harness
  does alike (stand-ins, the game's modules required, the world every scenario shares); where it
  returns is the checkpoint. Then the same Context gets those fields and `Run` goes on with it,
  mounting what the run varies (a character's bundle passed in the context, say) with `Context.Mount`. A
  service's prepared run of such a harness serves every job whose other arguments are the same:
  every character, item and scenario. A `Prepare` that yields prepares nothing.
- Frame 0 is sampled once the harness has started; then the clock runs `--fps` frames a second for
  `--seconds` (`--mode play`, the default: `RunService:IsRunning()` is true, one Heartbeat and one
  render step per frame, `Stepped` fires; `--mode edit` runs the edit-mode clock). A script that prints a
  line that is exactly `@done` ends the run at the end of that frame: the summary's `frames` are the
  frames that ran, and it says `"done": true`.

## Run context

- `--run-context server` runs the scripts as a Studio Play-mode server (docs/engine.md "Server run
  context"): `RunService:IsServer()` true and `IsClient()` false, no local player (Players starts empty;
  a harness parents `Instance.new("Player")`s to it), nothing renders, in-memory DataStoreService,
  MemoryStoreService and MessagingService that start empty every run, and EncodingService (Zstd,
  Base64); `FireClient` / `FireAllClients`
  reach no client and `InvokeClient` waits until its player leaves. It needs `--mode play`. The default,
  `client`, is a playing client with its local player, as before. `--studio false` makes
  `RunService:IsStudio()` false, as on a live game's server or client, so the game takes its live
  branches (TeleportData from `Player:GetJoinData()`, live costs and cooldowns); the default is true.
  It is part of a prepared run's key.

## Engine options

- Options take `--key value` or `--key=value` (the form for a value below zero). Engine options
  (src/launch.rs): `--roots "<dir>;<dir>"` (the project's Rojo roots; without them there is no project,
  only the harness and its mounts), `--project <dir>` (the project's folder) and `--place <name>`;
  `--mount Service/Name=<dir|file>` and `--instance Service/Name=<ClassName>` (repeatable),
  `--snapshot-dir <dir>` / `--snapshot <Service.rbxm>` (Studio exports; the checkout's `data/place-content/<Place>/<parent path>/*.rbxm` replaces a hierarchy snapshot's name-and-class placeholders with the stored instances, no flag), `--run-context client|server`, `--player <name>`,
  `--place-id <n>`, `--streaming <radius>` (the place streams: Workspace.StreamingEnabled and its
  StreamingTargetRadius, as rur.json's `streamingTargetRadius` sets them per root), `--viewport WxH`,
  `--offline`, `--verbose`. Content naming a local file (`file://<path>`, or a path such as
  `output/x.mesh`, as a game's asset id modules can name a mesh or image until it is uploaded:
  CreateMeshPartAsync, CreateEditableMeshAsync, CreateEditableImageAsync) reads a relative path from the
  project's folder: `--project`, else the folder holding the first root, else the current directory (a
  run of mounts alone; through `--service` its client names that as `--project`). `--assets-at
  <Service.Path>` and `--assets <prefix>,...` choose which asset manifest bundles mount where (over the
  project's rur.json `modelAssets`; `*` is every bundle; docs/engine.md "Asset bundles").
  Bundles come from an asset worker, the executable `REE_ASSET_WORKER` names, which downloads
  the assets the manifest pins (docs/engine.md "Asset bundles").

## Limits and guards

- A run cannot outlive its job (src/watchdog.rs). Roblox's script execution timeout: a resumption that runs 10 s of the
  work clock without yielding errors with `Script timeout: exhausted allowed execution time` at its next safepoint in
  script code, and at every one after until it yields or ends, so a `pcall` cannot keep it going. The work clock counts the
  VM's instructions, calls and allocations and the engine calls at Studio's prices, so the timeout fires under
  `REE_FROZEN_CLOCK=1` too, and on a busy machine as on an idle one; 60 s of host time in one resumption fires it as well. `--script-timeout <s>` /
  `REE_SCRIPT_TIMEOUT` (0: off). Each one writes `ree: script timeout <n> at <stack>` to stderr; the third stops the run:
  the frame loop ends it, stdout has the summary (status `error`, the failure names the stack) and ree exits 126
  (`--max-script-timeouts <n>` / `REE_MAX_SCRIPT_TIMEOUTS`, 0: never). A wall-clock cap, 600 s plus 20 s per `--seconds`,
  exits 124 (`--max-wall-seconds <s>` / `REE_MAX_WALL_SECONDS`, 0: off), and ree exits 125 when the process that started
  it is gone, at once when it is gone already (`--no-parent-watch` / `REE_PARENT_WATCH=0`). The `--service` client keeps
  the cap and the parent watch itself; the service's job has the timeout and returns its 126, and lives no longer than
  its client: a job whose client left while it waited (it exited, was ended, or gave up at its cap) is not run, and a
  running one stops before its next frame once its client's connection has closed (exit 125, no files written).
- What a script exhausts is that script's error and the run goes on: past 900,000 threads and values held for scripts
  (mlua's reference stack holds a million) task.spawn / defer / delay and the calls like them raise
  `<call>: too many threads and values held for scripts (...)` at the caller, and waits past 950,000 (docs/engine.md
  "Held values"); HttpService:JSONEncode refuses a cyclic table and nesting past 256 levels; the command runs on a 64 MiB
  stack, as the service's workers do.

## What a recording holds

- `record` writes `scene.rbxm` (every instance that was ever in the world, as it was when it appeared,
  hidden until then and after it leaves), `playback.json` (ror's playback: appearances and removals,
  `set` actions and keyed tweens, motion tracks, `Emit(n)` / `Clear()` (each emit with where its
  emitter was at the call), moves as `parent`, instance references written after an instance appeared
  as `set` actions naming the referenced instance's path, Lighting),
  `events.json` (prints, warnings, errors, sound plays and stops with their times), `camera.json` and
  `summary.json`, which is also printed on stdout; a `ree: record ...` line goes to stderr.

## Animation

- AnimationTracks play (docs/engine.md "Animation"): `Animator:LoadAnimation` (and the Humanoid's and
  AnimationController's) loads the KeyframeSequence its AnimationId names, and each play-mode frame
  the Animator writes the blended pose into the rig's Motor6D / Bone Transforms. Keyframes come from
  `--animations` directories (`<id>.rbxm` / `.rbxmx`), the download cache (`<state dir>/animations`)
  or the asset worker (its `asset` command); an id
  none of them has is warned about once and its tracks have Length 0. As in Studio, an asset's first
  request in a run arrives 0.45 s later (its tracks keep Length 0 and no pose until then; a later one,
  or one ContentProvider:PreloadAsync waited for, loads at once).
  `KeyframeSequenceProvider:GetKeyframeSequenceAsync` returns a copy of the same KeyframeSequence (it
  raises for an id none of them has). CurveAnimations and the default `Animate` script are not
  emulated.

## What is not simulated

- Nothing else is simulated: particles, trails and beams are what ror draws from the emits and switches;
  physics does not run beyond free fall; spatial queries see Block parts as boxes, Ball parts as
  spheres, Cylinder parts as cylinders and anything else as its oriented bounding box. Rigid joints
  (Weld, Motor6D with its Transform, WeldConstraint, ...) do carry the parts they join, as Roblox's
  kinematics do: moving a rig's HumanoidRootPart moves its body (docs/engine.md, "World and recorder").

## The warm service

- `--service` runs the command in `ree serve`, a warm process of this build and environment that
  keeps the reflection database, the project's module sources and bytecode and the converted
  manifest bundles between recordings (each recording still gets its own engine, VM and
  DataModel; ~0.25 GB private while idle). It runs several recordings at once, so callers sharing it
  do not queue: `REE_SERVE_WORKERS`, else one per 4 hardware threads and per 4 GB of memory, 1 to 4
  (`ree serve --workers <n>` when run by hand); a recording does not start beside others while less
  than 2 GB of memory is free. The client prints what the command prints and exits
  with its code. A lone command runs in its client (a warm service saved a sim nothing measurable,
  and starting one costs ~0.4 s); a command within 10 s of the last one of this build and environment
  (a batch, `@align` passes, parallel lanes) starts the service (detached, from a copy of the binary in `<state dir>/bin`, so cargo can rebuild
  `ree` meanwhile; a newer build replaces it), and the commands after it use it while it runs. It runs in its state dir, not the folder it was started
  from (a process holds its folder: a worktree could not be removed), and exits 20 s after its last
  job (`--idle`; health requests and runs prepared ahead are not jobs; a job whose client has gone stops, so it holds
  no service up: /health counts it `abandoned`). `ree serve --help` lists its options; one it does not know is refused. A service is
  found by its build and by the environment variables a run reads (`RUR_*`, `REE_*`, `ROR_*`), so a
  client with another environment gets its own. Without a reachable service
  the client runs the command itself, as it does for a Luau profile (`RUR_LUAU_PROFILE`). A record
  whose process is gone (a crash, a reboot) is skipped without connecting to it. After a recording
  (or a `ree run`) whose harness called `Context.Checkpoint()`, the worker runs the same command again
  in a fresh engine up to that call and holds it there (src/record/prepare.rs): the next job of that
  command with the same arguments (`--out` and `--profile` aside; for a harness with `Prepare`,
  `--context`, `--seconds` and `--fps` aside too; `--run-context` is one of them, so a client's and a
  server's never share one) goes on from it when everything the run read before the
  call reads the same (the files and listings it read, the project's directories, the wall clock's
  hour), checked while it goes on; else it starts over. It records the same bytes as a recording
  made from the start (a 3 s sim: ~290 -> ~35 ms in the service; with the harness's `Prepare`, for
  every character and scenario). A run is prepared only when jobs like it repeat
  (another started within 10 s, or one waits) and its job read nothing a prepared run cannot check
  before the checkpoint (a mesh, an image: the preparation would fail alike); a preparation that fails
  is not tried again until the next job like it. A worker holds one
  such run (~0.2 GB); one for other arguments is dropped after the next job's reply. An idle worker
  holding nothing prepares the most recent recording too while fewer than two workers hold it, so
  the next job like it finds one ready while the other is prepared again. A job waits for a worker
  still preparing its run only when that is expected ready within 0.2 s; otherwise a free worker
  records it from the start (and then prepares one too). The service listens on 127.0.0.1 only and
  needs an `X-REE: 1` header and a local Host; it has no authentication, so it is a tool for a
  single-user machine (SECURITY.md).
- `ree serve --warm [record's engine options]` starts the service of this build and environment when
  it is missing (run it with the environment the recordings use) and reads in what
  a first recording of the project reads: the last module lists (`record/*.txt`) of the harnesses run
  against this project (their files in its roots or its `external` data roots) read and compiled, the
  plans of the bundles recent recordings of that mount imported, the project scan, the Studio hierarchy
  and Engine.Start; a list whose checkout is gone (its first file's folder is) is deleted on the way.
  It records nothing, writes only caches in the state dir,
  prints a JSON summary (`{"warm":true,"modules":...,"modulesRead":...,"bundles":...,"bundlesRead":...,"seconds":...}`:
  what is ready, and what of it was read now) and exits 0 once
  the service is ready (1 when it could not reach one); the service then idles out as usual. A build
  without it exits 2 with `ree serve: unknown option --warm`.

## Profiles and caches

- `--profile` prints where the command's time went on stderr: the mount phases, the harness start,
  module loads, bundle imports, the frames and the files (`prepared: used`, or why a prepared run was
  stale, when a service's worker held one).
- Caches in the state dir: `animations/` (downloaded animation assets, `<id>.rbxm`; delete a file to
  fetch it again), `model-bundles/` (bundle files as uploaded, `asset-<AssetId>-v<Version>.rbxm`, named by
  the asset version they hold and shared by every project that pins it, and the manifest's catalog: the
  worker starts only for a bundle the manifest pins anew; beside each, its
  converted plan, `<file>.plan`, valid while the conversion is the same (build.rs digests its code and
  data: a rebuild that changes neither keeps it): a run maps it and reads it in place (Windows; the
  pages are the file's, shared by every run reading it) instead of decoding and converting the
  bundle again),
  `record/<harness and project>.txt` (the module files a harness's last run read: the next run reads
  and compiles them on other threads), `model-bundles/recent-*.txt` (the bundles the last run of a
  mount imported, and those of the last run of each scene, a harness with its place and context: the
  next run of the scene decodes them ahead the same way; its first run decodes the mount's), `services/`, `bin/` and `serve.log` (the service). All are guesses that only
  save time: a stale one is noticed and the file is read as usual.

## Environment

Environment (the variables keep the names they had in rur, whose tools set them; the guards' `REE_SCRIPT_TIMEOUT`,
`REE_MAX_SCRIPT_TIMEOUTS`, `REE_MAX_WALL_SECONDS` and `REE_PARENT_WATCH` are above; docs/engine.md "Environment"
lists them all): `RUR_STATE_DIR`
(caches; default `.cache`), `RUR_VERBOSE=1` (script output on stderr), `RUR_ENGINE_TIMINGS=1` (mount
phases), `RUR_ENGINE_DUMP=<file>` (every instance after the mount), `RUR_EPOCH` (the wall clock's start),
`RUR_CLOCK` (a recorded Studio clock), `RUR_CAMERA`, `RUR_PLACE_ID`, `RUR_PLAYER`, `RUR_PLAYER_ID`,
`RUR_PLAYER_DISPLAY`, `RUR_ROTATION_CLIPPING`, `RUR_PROFILE_CALLBACKS`, `RUR_LUAU_PROFILE=<file>` (the
Luau VM's sampled stacks, folded; `RUR_LUAU_PROFILE_CLOCK=work` samples the work clock's charges
instead of host time, `RUR_LUAU_PROFILE_UNIT=us` writes microseconds), `RUR_LUAU_ALLOC_PROFILE=<file>` (bytes the VM allocates by Luau stack,
folded, with the allocation counts in `<file>.counts`; a native function's allocations end the stack in it;
`RUR_LUAU_ALLOC_STEP=<bytes>` sets the sampling step, default 1024, 1 records every allocation; a harness
names the phase of both profiles with `Context.ProfilePhase(Name)`; see src/luau_profile.rs),
`REE_LOCALIZED_PLACES` / `REE_MATCH_PLACES` (places whose clients pay a localization lookup on Text
writes, or more for part geometry writes: docs/engine.md "Work clock").
