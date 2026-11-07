# Verification

What checks the engine, and what each check covers.

- `cargo test --release` here: the engine's unit tests (ports of the former Luau runtime's tests,
  script-API probes, the world and recorder, the Studio-measured behaviours they pin) and the patched
  VM's (`tests/luau_vm.rs`). No test needs Studio, a game checkout, a GPU or the network.
- Downstream, roblox-ui-renderer's golden tests run its fixtures and Studio comparison replays through
  this engine and require their output to stay identical to Studio's recordings.
- Differential probes against the real engine (`tools/parity`, its README): Luau probes of the scheduler,
  signals, instances, the Luau library and datatypes run in ree and in a Roblox client, and
  `node tools/parity/run.mjs` prints which lines differ. Their captured output is checked by
  `cargo test --release --test parity`: a line that matched the real engine must keep matching; the
  differences not fixed yet are listed, with their reasons, in `tools/parity/known.json`. Probes that need a
  GUI layout (when Absolute* properties and CanvasPosition change and fire during a scroll) run through
  roblox-ui-renderer with `--rur <checkout>`.
- Recordings: `ree record` of the same harness and project gives byte-identical `scene.rbxm`,
  `playback.json`, `events.json`, `camera.json` and summary.
