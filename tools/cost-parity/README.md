# The parity bench: ree's work clock against a real client

`os.clock` in ree is a work clock (docs/engine.md "Work clock", src/cost.rs): every engine call and every kind of
Luau work is charged a price. This bench measures how far those prices are from a real client's: about 400
workloads run unchanged in ree and in a Roblox client, and each one's time per operation is compared.

- `Corpus.luau`: the engine-call workloads by family (reads, writes, methods, finds, attributes, Instance.new,
  Clone, reparenting, Destroy, signals, threads, tweens, datatypes, library calls, garbage, memory), a scene
  they share, and the runner (median of 9 rounds; a round's own measure where the work runs later, such as a
  signal's handlers).
- `Language.luau`: pure Luau loops. The job loads it three times: as is, with `--!native` (a client compiles
  that module to machine code) and after a `getfenv()` call (whose environment is then unsafe: imports are
  looked up again and builtins become C calls), which runs only the rows `M.Unsafe` names.
- `parity.py`: runs either side, compares, prints the table.
- `fit.py`: the prices that would make rows exact (`residuals`), and fits of the Luau prices by kind of
  instruction (`luau`, `luau --native`) and of the library's (`library`).

A third of the workloads, chosen by a hash of their names (`parity.held_out`), is held out: no price may be
fitted on it, and the table reports it apart. `parity.py compare --fit-rows` lists the rows a price may use.

## Running

```sh
python -I tools/cost-parity/parity.py ree --ree target/release/ree.exe --out ree.json     # ree's side (REE_WORK_COUNTERS=1)
python -I tools/cost-parity/parity.py ree --device phone --out ree-phone.json              # with the phone price profile
python -I tools/cost-parity/parity.py job > job.luau                                       # the engine side's job text
python -I tools/cost-parity/parity.py compare ree.json engine.json [engine2.json ...]      # the table
python -I tools/cost-parity/fit.py residuals ree.json engine.json --family method
python -I tools/cost-parity/fit.py luau ree.json engine.json [--native]
```

The engine side is any client that can set `ModuleScript.Source` and require it (identity 4: a plugin-level
command bar in a playtest client, or a job), running the job text and returning its value as JSON. Run the
whole corpus in each session: what a client has done before moves some prices (a heap of a game's size makes
new strings 2-3x dearer, and spatial queries after many parts were made and moved cost 3x), and the reference
numbers come from sessions that ran everything. Several engine files give the noise: each row's spread
between sessions is printed beside its error.


## Reading the table

`compare`'s header names ree's commit and device and each engine run's version and place. Each row gives the
engine's and ree's time per operation (ns, or bytes for the `mem` rows), ree / engine, and the spread between
engine sessions; then the fit rows, the held-out rows and all rows: the median, mean and worst absolute error
and how many rows are within 25 % and 50 %, and the median error by family. A price change lists the rows it
moves, the error on the fit rows and on the held-out rows apart.

The `meta` of each side's output records what the corpus found: the engine's version, signal behaviour, the
place's localization tables, the compiler's choices (a constant vector's `typeof`), and what the spatial
rows' queries return on each side (both sides must find the same parts for a per-part price to compare).
