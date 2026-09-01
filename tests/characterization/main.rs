//! Characterization tests for the Bash CLI in `bin/oqtopus`.
//!
//! These tests pin down the CLI's observable behavior — exit codes, exact
//! stdout/stderr text, resulting files, and the argv passed to external
//! tools — as insta snapshots. They exist so a reimplementation (e.g. a Rust
//! port, selected via `OQTOPUS_TEST_BIN`) can be validated against the Bash
//! baseline byte for byte. See README.md in this directory for the layout,
//! the fixture model, and how to review snapshot changes.

#[path = "../support/fake_tool.rs"]
mod fake_tool;
mod harness;

mod backend;
mod errors;
mod help;
mod init;
mod manager;
mod metadata;
mod versions;
