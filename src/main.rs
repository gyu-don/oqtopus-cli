//! OQTOPUS command-line entry point.
//!
//! Commands are being migrated incrementally from the legacy Bash implementation. This crate
//! handles migrated routes directly and replaces itself with the legacy CLI for all other routes,
//! preserving command-line compatibility during the transition.

use std::env;
use std::ffi::{CString, OsStr, OsString};
use std::fs;
use std::io::{self, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{self, Command};

const FORBID_LEGACY_FALLBACK: &str = "OQTOPUS_FORBID_LEGACY_FALLBACK";
/// Exit status used when a caller requires Rust-only execution but routing reaches the legacy CLI.
///
/// This distinguishes a deliberately blocked fallback from an ordinary command failure.
const FALLBACK_FORBIDDEN_EXIT_CODE: i32 = 125;

const TOP_LEVEL_HELP: &str = "\
Usage:
  oqtopus <command> [args]

Commands:
  init         Create an OQTOPUS environment.
  cloud-local  Manage local cloud-local components and services.
  backend      Manage local backend components and services.
  manager      Manage the local manager component and service.
  completion   Print shell completion scripts.
  version      Print the installed CLI version.
  help         Show help.

Run 'oqtopus <command> help' for command-specific help.
";

/// Implementation selected for a command-line invocation.
///
/// Routing is intentionally coarse while the Rust migration is in progress: anything not listed
/// here remains the legacy CLI's responsibility.
enum Route {
    Help,
    Version,
    BackendInfo,
    Legacy,
}

/// Selects the Rust implementation for migrated commands and [`Route::Legacy`] otherwise.
fn route(args: &[OsString]) -> Route {
    // Match OsStr values directly so non-UTF-8 arguments can still be forwarded unchanged to Bash.
    match args.first().map(OsString::as_os_str) {
        None => Route::Help,
        Some(command) if command == OsStr::new("help") || command == OsStr::new("--help") => {
            Route::Help
        }
        Some(command) if command == OsStr::new("version") || command == OsStr::new("--version") => {
            Route::Version
        }
        Some(command)
            if command == OsStr::new("backend")
                && args.get(1).is_some_and(|arg| arg == OsStr::new("info")) =>
        {
            Route::BackendInfo
        }
        Some(_) => Route::Legacy,
    }
}

/// Returns the first value for `key` from the environment's line-oriented metadata format.
fn metadata_get<'a>(contents: &'a str, key: &str) -> Option<&'a str> {
    // Split only once because metadata values may themselves contain '='.
    contents.lines().find_map(|line| {
        let (candidate, value) = line.split_once('=')?;
        (candidate == key).then_some(value)
    })
}

fn metadata_set(contents: &str, key: &str, value: &str) -> String {
    // Preserve ordering and unknown lines so migrating one key does not rewrite metadata owned by
    // other components.
    let mut found = false;
    let mut updated = String::new();

    for line in contents.lines() {
        if line
            .split_once('=')
            .is_some_and(|(candidate, _)| candidate == key)
        {
            updated.push_str(key);
            updated.push('=');
            updated.push_str(value);
            found = true;
        } else {
            updated.push_str(line);
        }
        updated.push('\n');
    }

    if !found {
        updated.push_str(key);
        updated.push('=');
        updated.push_str(value);
        updated.push('\n');
    }

    updated
}

fn metadata_unset(contents: &str, key: &str) -> String {
    let mut updated = String::new();

    for line in contents.lines() {
        if !line
            .split_once('=')
            .is_some_and(|(candidate, _)| candidate == key)
        {
            updated.push_str(line);
            updated.push('\n');
        }
    }

    updated
}

fn migrate_key(contents: String, old_key: &str, new_key: &str) -> String {
    let Some(value) = metadata_get(&contents, old_key).map(str::to_owned) else {
        return contents;
    };

    let contents = metadata_set(&contents, new_key, &value);
    metadata_unset(&contents, old_key)
}

/// Atomically replaces `path` with newly created, owner-writable contents.
fn replace_file(path: &Path, contents: &[u8]) -> io::Result<()> {
    // Create and sync a randomly named sibling before rename so readers never observe partially
    // migrated metadata and a stale file cannot block a later process that reuses the same PID.
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .unwrap_or_else(|| OsStr::new("oqtopus"))
        .to_string_lossy();
    let mut temporary = tempfile::Builder::new()
        .prefix(&format!(".{file_name}.tmp."))
        .tempfile_in(parent)?;

    temporary.write_all(contents)?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}

/// Checks write access using the process's real user and group IDs.
///
/// This matches Bash's `[[ -w path ]]` for ordinary invocations where real and effective IDs
/// coincide. Invocations with differing real and effective IDs are outside this check's contract.
fn is_writable(path: &Path) -> bool {
    let Ok(path) = CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };

    // SAFETY: `path` is a valid, NUL-terminated C string and remains alive for the call.
    unsafe { libc::access(path.as_ptr(), libc::W_OK) == 0 }
}

/// Migrates legacy metadata keys when `path` can be safely rewritten.
///
/// Migration is opportunistic: callers remain usable for read-only environments and should report
/// metadata validation errors rather than failing solely because the compatibility rewrite failed.
fn migrate_metadata_keys(path: &Path) {
    if !is_writable(path) {
        return;
    }

    let Ok(contents) = fs::read_to_string(path) else {
        return;
    };
    let migrated = migrate_key(contents.clone(), "env_root", "environment_root");
    let migrated = migrate_key(migrated, "env_name", "environment_name");

    if migrated != contents {
        let _ = replace_file(path, migrated.as_bytes());
    }
}

/// Validates the current backend environment and returns its canonical metadata for display.
fn backend_info(args: &[OsString]) -> Result<Vec<u8>, String> {
    if !args.is_empty() {
        return Err("oqtopus backend info does not accept arguments.".to_owned());
    }

    let path = Path::new(".metadata");
    if !path.is_file() {
        return Err(
            ".metadata not found.\nThis directory is not an OQTOPUS backend environment."
                .to_owned(),
        );
    }

    migrate_metadata_keys(path);
    // Parse through a lossy view, but retain the original bytes for output. This preserves the
    // legacy command's byte-for-byte behavior after the fields required for validation are found.
    // The legacy metadata lookup also reports an unreadable file as a missing required key.
    let contents = fs::read(path).map_err(|_| "invalid .metadata: missing template.".to_owned())?;
    let text = String::from_utf8_lossy(&contents);

    let template = metadata_get(&text, "template")
        .ok_or_else(|| "invalid .metadata: missing template.".to_owned())?;
    if template != "backend" {
        return Err(format!(
            "invalid environment template. Found template='{template}', but 'oqtopus backend' requires template='backend'."
        ));
    }

    let environment_root = metadata_get(&text, "environment_root")
        .or_else(|| metadata_get(&text, "env_root"))
        .ok_or_else(|| "invalid .metadata: missing environment_root.".to_owned())?;
    let current = fs::canonicalize(".")
        .map_err(|error| format!("failed to resolve current directory: {error}"))?;
    if environment_root != current.to_string_lossy() {
        return Err(format!(
            "Current directory does not match environment_root.\nenvironment_root = {environment_root}\ncurrent          = {}",
            current.display()
        ));
    }

    metadata_get(&text, "install_root")
        .ok_or_else(|| "invalid .metadata: missing install_root.".to_owned())?;

    Ok(contents)
}

/// Replaces the current process with the legacy CLI, forwarding arguments unchanged.
fn run_legacy(args: &[OsString]) -> ! {
    if env::var_os(FORBID_LEGACY_FALLBACK).is_some() {
        eprintln!("Error: legacy Bash fallback is forbidden for this invocation.");
        process::exit(FALLBACK_FORBIDDEN_EXIT_CODE);
    }

    let legacy_cli = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("bin/oqtopus");
    // exec replaces this process, preserving the legacy CLI's exit status and signal behavior.
    let error = Command::new("bash").arg(legacy_cli).args(args).exec();

    eprintln!("Error: failed to run the legacy OQTOPUS CLI: {error}");
    process::exit(126);
}

fn main() {
    // Rust ignores SIGPIPE by default. CLI pipelines expect the traditional Unix behavior: exit
    // silently when a downstream reader such as `head` or `grep -q` closes the pipe early.
    // SAFETY: installing the default disposition for SIGPIPE requires no Rust-managed callback.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }

    let args: Vec<_> = env::args_os().skip(1).collect();

    match route(&args) {
        Route::Help => print!("{TOP_LEVEL_HELP}"),
        Route::Version => println!("oqtopus {}", env!("CARGO_PKG_VERSION")),
        Route::BackendInfo => match backend_info(&args[2..]) {
            Ok(contents) => {
                if let Err(error) = io::stdout().write_all(&contents) {
                    eprintln!("Error: failed to write backend info: {error}");
                    process::exit(1);
                }
            }
            Err(error) => {
                eprintln!("Error: {error}");
                process::exit(1);
            }
        },
        Route::Legacy => run_legacy(&args),
    }
}
