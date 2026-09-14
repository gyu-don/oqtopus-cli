//! OQTOPUS command-line entry point.
//!
//! Commands are being migrated incrementally from the legacy Bash implementation. This crate
//! handles migrated routes directly and replaces itself with the legacy CLI for all other routes,
//! preserving command-line compatibility during the transition.

mod archive;
mod args;
mod backend;
mod cli;
mod cloud_local;
mod environment;
mod init;
mod legacy;
mod lifecycle;
mod manager;
mod metadata;
mod operations;
mod progress;
mod remote;
mod service;
mod text;
mod version;
mod versions;

use std::env;
use std::process;

fn main() {
    // Rust ignores SIGPIPE by default. CLI pipelines expect the traditional Unix behavior: exit
    // silently when a downstream reader such as `head` or `grep -q` closes the pipe early.
    // SAFETY: installing the default disposition for SIGPIPE requires no Rust-managed callback.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }

    let args: Vec<String> = env::args().skip(1).collect();
    let exit_code = cli::run(&args);
    if exit_code != 0 {
        process::exit(exit_code);
    }
}
