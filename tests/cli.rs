//! CLI integration tests through the built `ree` binary: process guards, profiling pass isolation, and what-if comparisons.
//! Merged into one binary so all subprocess tests compile once and run concurrently under one libtest harness.

#[path = "cli/process_guards.rs"]
mod process_guards;

#[path = "cli/profile_passes.rs"]
mod profile_passes;

#[path = "cli/what_if.rs"]
mod what_if;
