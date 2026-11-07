# Parity probes: ree against the real engine

Differential tests of what scripts see. Each probe in `probes/` is a Luau body that runs unchanged in ree and in a
Roblox client and prints canonical lines; `expected/` holds what the real engine printed, and the runner compares.

```sh
node tools/parity/run.mjs                 # every probe through target/release/ree: a pass/diff table
node tools/parity/run.mjs task_order --diff   # one probe, with its differing lines (- real engine, + ree)
node tools/parity/run.mjs --write-known   # record the current differences in known.json (keeps their reasons)
REE_PARITY_CAPTURE="<command>" node tools/parity/run.mjs --capture [probe ...]   # re-capture after a Roblox update
```

`cargo test --release --test parity` runs the same comparison: every expected line ree does not print must be listed in
`known.json`, so a change that breaks a matching line fails the tests. The table's columns are the expected lines, the
ones ree printed the same, the known differences and the new ones; it also names `known.json` entries that now match
(remove them).

## Writing a probe

- `prelude.luau` comes first: `P.num` / `P.f32` print numbers the same in every engine, `P.err` takes the probe's own
  chunk name and line offset out of an error (`@12:` is the probe's line 12), `P.try` prints a call's outcome, `P.rng`
  draws cases from a generator of its own (Random and math.random are under test), `P.sync` starts at a scheduler
  resumption point. `print` escapes invalid UTF-8 and control bytes.
- Print nothing that varies between runs of the real engine: addresses, timings, frame-time sums. Count frames for
  waits, and keep durations off frame boundaries (0.05 s is exactly three 60 Hz frames; use 0.04 or 0.06).
- A headless client runs its render step on its own cadence (none, or two, between simulation steps): keep
  PreRender-relative order out of lines that must be stable.
- A probe that needs another script identity says so in a line `-- parity: identity=4` (writing `Source` and
  requiring the module needs 4 in a client).
- Capture twice and compare the two before trusting a new probe: a line that changes between real runs is a probe bug.

## Capturing

The expected files name the engine and date in their `# ` header lines. `--capture` runs the bodies through the
command `REE_PARITY_CAPTURE` names, called as `<command> <bodies.json> <outputs.json>`: the input is
`[{"name", "body"}]`, and the command writes `{"engine", "date", "probes": {"<name>": {"ok", "lines", "error"}}}`
with each body's printed lines. It must run each body as a function body in a playing client whose
`Workspace.SignalBehavior` is Deferred (ree's model), at script identity 2 unless the probe asks for another.

