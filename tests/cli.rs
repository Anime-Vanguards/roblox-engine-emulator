//! CLI integration tests through the built `ree` binary: process guards, profiling pass isolation, what-if comparisons and
//! client profiles in a batch.
//! Merged into one binary so all subprocess tests compile once and run concurrently under one libtest harness.

#[path = "cli/process_guards.rs"]
mod process_guards;

#[path = "cli/profile_passes.rs"]
mod profile_passes;

#[path = "cli/what_if.rs"]
mod what_if;

#[path = "cli/client_profile.rs"]
mod client_profile;
