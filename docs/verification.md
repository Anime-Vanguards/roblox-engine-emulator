# Verification

What checks the engine, and what each check covers.

- `cargo test --release` here: the engine's unit tests (ports of the former Luau runtime's tests,
  script-API probes, the world and recorder, the Studio-measured behaviours they pin) and the patched
  VM's (`tests/luau_vm.rs`). No test needs Studio, a game checkout, a GPU or the network.
- Downstream, roblox-ui-renderer's golden tests run its fixtures and Studio comparison replays through
  this engine and require their output to stay identical to Studio's recordings.
- Recordings: `ree record` of the same harness and project gives byte-identical `scene.rbxm`,
  `playback.json`, `events.json`, `camera.json` and summary.
