//! Profiles built for the viewer tests: cost model rules, hardware counters, issue detectors, and headless views.
//! Merged into one test binary so the shared fixture builder compiles once instead of four times.

mod common;

#[path = "viewer/fixture.rs"]
mod fixture;

#[path = "viewer/cost_model.rs"]
mod cost_model;

#[path = "viewer/counters.rs"]
mod counters;

#[path = "viewer/issues.rs"]
mod issues;

#[path = "viewer/views.rs"]
mod views;
