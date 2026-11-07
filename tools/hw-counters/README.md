# ree-hw-counters

The Windows service `ree ... --profile hw` reads the CPU's hardware counters through: a process cannot read them
itself, and the kernel trace session that can needs the system's profiling right, which an account's ordinary
processes never hold. Installed once from an elevated PowerShell (`& .\setup.ps1`; `-Uninstall` removes it), it counts
for the one account it was installed for, only that account's processes' own threads.

What it grants, the counters, how the counts are booked and how to check them: docs/cli.md "Hardware counters". Its
wire format and client are ree's own (src/profiler/hw), shared by path.

```text
cargo test --release --manifest-path tools/hw-counters/Cargo.toml    # its tests (one reads a real trace session)
ree-hw-counters probe     # every counter against kernels whose counts are known, through the installed service
ree-hw-counters sources   # the counters this CPU offers
```
