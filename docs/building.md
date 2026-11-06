# Building

How to build ree, the cargo features and the allocator. The [README](../README.md) has the short version.

## Requirements

- Rust 1.90 or later (edition 2024).
- A C++ toolchain for the Luau VM (on Windows, the MSVC Build Tools). `.cargo/config.toml` links the
  Windows target with rust-lld, which comes with Rust.
- Git, for the `ror-mesh` dependency (feature `meshes`), fetched from the public
  [roblox-object-renderer](https://github.com/Anime-Vanguards/roblox-object-renderer) repository.
- Node.js 20 or later, only to re-vendor the VM (`tools/vendor.mjs`).

## Build

```sh
cargo build --release        # target/release/ree(.exe)
cargo test --release         # the engine's tests and the patched VM's
```

Nothing else is fetched: the patched Luau VM (`vendor/luau0-src`) and Lune's Roblox datatypes
(`vendor/lune-roblox`, `vendor/lune-utils`) are vendored. They are the pinned upstream archives with
`patches/luau.patch` and `patches/lune.patch` applied: `node tools/vendor.mjs` rebuilds `vendor/` after
a patch changes, `node tools/vendor.mjs --check` verifies it. `--release` is the everyday build:
incremental, linked with rust-lld (`.cargo/config.toml`), and with cargo's ThinLTO across the crate's own
codegen units (`lto = false`), which keeps the engine's cross-module inlining. Measured 2026-10-10:
`lto = "off"` rebuilt a one-line edit in 7 s instead of 12 s, but property reads and writes ran 5-25%
slower, so the profile keeps it. Float results are the same either way.

Features: `jit` (Luau native code generation), `bundles` (asset manifest bundles through an asset
worker process), `meshes` (EditableMesh from mesh assets, decoded by ror's `ror-mesh`) and `cli` (the
`ree` binary), all on by default; a WebAssembly host turns off `jit` and `bundles`. `dhat-heap` builds a
`ree` that writes a heap profile of its command (or of `ree serve` when it exits) to `dhat-heap.json`,
for DHAT's viewer: `cargo build --release --features dhat-heap --target-dir target-dhat`.

`ree` allocates with mimalloc: v2, which hands a finished run's pages back where v3 kept them
committed, committing a thread's segments page by page (`MIMALLOC_EAGER_COMMIT=1` restores whole
segments). What a warm service keeps across runs (converted bundles, compiled modules, the Studio
hierarchy) is copied to a heap of its own while `alloc::long_lived` runs, so the pages a run used are
freed whole when it ends (src/alloc.rs).
