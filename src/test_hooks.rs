//! Environment overrides that let the characterization suite replace network access, wall-clock
//! time, and port probing with fixed fixtures.
//!
//! The overrides exist only in builds with the `test-hooks` feature, so a release binary ignores
//! every `OQTOPUS_TEST_*` variable. Even in a test build they stay inert unless
//! `OQTOPUS_TEST_MODE` is set.

use std::ffi::OsString;

/// Returns the value of the test override `name`, or `None` when hooks are unavailable.
#[cfg(feature = "test-hooks")]
pub(crate) fn var_os(name: &str) -> Option<OsString> {
    std::env::var_os("OQTOPUS_TEST_MODE")?;
    std::env::var_os(name)
}

#[cfg(not(feature = "test-hooks"))]
pub(crate) fn var_os(_name: &str) -> Option<OsString> {
    None
}

/// Returns the UTF-8 value of the test override `name`; a non-UTF-8 value is ignored.
pub(crate) fn var(name: &str) -> Option<String> {
    var_os(name)?.into_string().ok()
}
