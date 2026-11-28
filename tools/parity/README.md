# Parity probes (`tools/parity`)

Differential tests comparing `ree` directly against a real Roblox client or server. Each script in `probes/` runs unmodified on both engines and prints canonical output lines; `expected/` stores the real engine's output, and the runner diffs them.

## Commands

```sh
node tools/parity/run.mjs                                # Run all client probes and print pass/diff table
node tools/parity/run.mjs task_order --diff              # Show line-by-line diff (- real engine, + ree)
node tools/parity/run.mjs --write-known                  # Update known.json with current diffs (preserves reasons)
node tools/parity/run.mjs ui_signals --rur <rur-dir>     # Run probes requiring GUI layout via rur
node tools/parity/run.mjs --server                       # Run server-side probes (expected/server, known-server.json)
node tools/parity/run.mjs --loopback                     # Run paired client+server probes (expected/loopback, known-loopback.json)

# Re-capture expected outputs after a Roblox update
REE_PARITY_CAPTURE="<command>" node tools/parity/run.mjs --capture [probe ...]
```

`cargo test --release --test parity` enforces the exact same checks across client, server, and loopback sets: any line that previously matched must keep matching, any remaining discrepancy must be recorded in `known*.json`, and newly matching entries are flagged so you can remove them from `known*.json`.

- **`--server`:** Runs eligible probes with `--run-context server` (skipping device/renderer probes).
- **`--loopback`:** Runs paired probes (`probes/loopback/`) with `--run-context loopback`, testing server+client interaction in one session.

## Paired loopback probes

Each `probes/loopback/<name>.luau` file contains shared code at the top, followed by `--- server` and `--- client` sections. Each side executes the file with the other section blanked out (preserving line numbers in `@line:` errors) after `prelude.luau` and `loopback.luau`:
- Server code boots before the player joins; client code runs once client scripts start.
- `P.player()` returns the local player on the client, or waits for the first player to join on the server.
- `P.meet(Tag)` synchronizes both sides at a barrier and yields to a scheduler resumption point. Frame counts are tracked per side and never compared across processes.
- At completion, the server forwards its collected lines to the client, which outputs `server: <line>` followed by `client: <line>`.
- Always capture loopback probes 3 times and keep only lines identical across all 3 runs—cross-process frame phase jitter makes single-send frame counts noisy.

## Animation assets

A probe that plays an animation names a real asset, which the real engine fetches. A parity run (`run.mjs`, `cargo test --test parity`) serves each such id from `animations/<id>.rbxmx` instead, through ree's `--animations`, so it needs no asset access: a clip made for the tests with what the probes read of the asset. `animations/507766666.rbxmx` stands in for Roblox's R15 idle (`http://www.roblox.com/asset/?id=507766666`, the form a server fetches too): a looped KeyframeSequence of 4.3 s that moves the `LowerTorso` joint.

- **On the real asset:** `cargo test --test parity` also runs these probes without the stand-ins where the machine fetches the asset (the download worker writes it into the run's cache), and says on stderr which it could not compare; `node tools/parity/run.mjs --real-assets [--server]` does the same by hand.
- **A new asset:** a probe on an asset that has no stand-in yet gets one in the same commit, with the Length, Looped and posed joints its lines depend on.

## Authoring a probe

- **`prelude.luau` helpers:** `P.num` / `P.f32` (portable number formatting), `P.err` (strips chunk names and normalizes line numbers to `@12:`), `P.try` (prints call outcome), `P.rng` (isolated RNG so `Random` / `math.random` can be tested), `P.sync` (aligns to a scheduler resumption point). `print` escapes invalid UTF-8 and control bytes.
- **Values made at run time:** the compiler folds the constant fields of a local table that never escapes as it folds literals, so `K.Sub * K.One` from `local K = { ... }` is a compile-time constant; build the table with `table.clone({ ... })` for run-time values (`float_flush.luau`).
- **Avoid nondeterminism:** Never print table addresses, wall timings, or raw frame-time sums. Count frames instead of relying on exact boundary durations (use `0.04` or `0.06` s rather than `0.05` s, which falls on 3 x 60 Hz frames). Headless clients tick `PreRender` independently of simulation steps, so don't rely on `PreRender`-relative ordering.
- **Directive comments:**
  - `-- parity: identity=4` — run at script identity 4 (needed to write `ModuleScript.Source`); `ree` runs its harness at that identity too (`--identity 4`), and the scripts it starts at a game script's.
  - `context=edit` — run `ree` in its edit run context (`--mode edit --run-context edit`, a plugin's code) for a probe captured at identity 4, whose Plugin capability a game script lacks (`settings()`). Such a probe has no server capture.
  - `timeout=<seconds>` — a probe captured with `ScriptContext:SetTimeout(<seconds>)` (at identity 4: a game script may not call it): `ree` runs it with `--script-timeout <seconds> --max-script-timeouts 0`. The probe calls `SetTimeout` itself in a `pcall` and sets it back to 0 (no timeout) at its end, as the capture session is shared.
  - `studio=false` — a probe of what a running game refuses that Studio allows (the capture's client and server are a running game): `ree` runs it with `--studio false`.
  - `flag=<name>` — enable an engine flag (e.g., `flag=FFlagEnableLoadModule` passes `--load-module` to `ree`). Server runs automatically pass `--load-string` to match server captures.
  - `topbar` — a probe of Roblox's top bar, captured at identity 2 on a rendered 1280 x 720 playing client running its core scripts (StarterScript), whose bar has a 58 px inset: `ree` runs it with the client profile `{"device":{"topbarInset":58,"viewport":[1280,720]}}`, and not on a server. `topbar=gamepad` is the same on a client whose only input is a gamepad (`{"device":{"kind":"gamepad",...}}`; the session's `--device-input gamepad`). The capture command runs no core scripts, so `--capture` leaves such a probe out; its expected file is captured by hand.
  - `-- parity: device=desktop rendered` — capture on a rendered 1920x1080 desktop client (58 px top-bar inset). Plain `ree` lacks GUI layout, so these lines sit in `known.json` and must match 100% when run with `--rur <checkout>` (`RUR_NATIVE_ENGINE` overrides the `rur-native` binary).
- Capture new probes twice and verify identical output before checking them in.

## Capture harness contract

`expected/*.txt` headers record the Roblox engine version and date. `--capture` executes `<REE_PARITY_CAPTURE> <bodies.json> <outputs.json>`:
- Input: `[{"name", "body", "server"?}]`.
- Output: `{"engine", "date", "probes": {"<name>": {"ok", "lines", "error", "engine"?}}}`.
- Run client bodies at identity 2 (unless overridden) in a play client with `Workspace.SignalBehavior = Deferred`; run `"server": true` bodies as a server `Script` in `ServerScriptService` at identity 2.

