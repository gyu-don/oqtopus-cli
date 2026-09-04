use std::env;
use std::ffi::{OsStr, OsString};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{self, Command};

const FORBID_LEGACY_FALLBACK: &str = "OQTOPUS_FORBID_LEGACY_FALLBACK";
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

enum Route {
    Help,
    Version,
    Legacy,
}

fn route(args: &[OsString]) -> Route {
    match args.first().map(OsString::as_os_str) {
        None => Route::Help,
        Some(command) if command == OsStr::new("help") || command == OsStr::new("--help") => {
            Route::Help
        }
        Some(command) if command == OsStr::new("version") || command == OsStr::new("--version") => {
            Route::Version
        }
        Some(_) => Route::Legacy,
    }
}

fn run_legacy(args: &[OsString]) -> ! {
    if env::var_os(FORBID_LEGACY_FALLBACK).is_some() {
        eprintln!("Error: legacy Bash fallback is forbidden for this invocation.");
        process::exit(FALLBACK_FORBIDDEN_EXIT_CODE);
    }

    let legacy_cli = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("bin/oqtopus");
    let error = Command::new("bash").arg(legacy_cli).args(args).exec();

    eprintln!("Error: failed to run the legacy OQTOPUS CLI: {error}");
    process::exit(126);
}

fn main() {
    let args: Vec<_> = env::args_os().skip(1).collect();

    match route(&args) {
        Route::Help => print!("{TOP_LEVEL_HELP}"),
        Route::Version => println!("oqtopus {}", env!("CARGO_PKG_VERSION")),
        Route::Legacy => run_legacy(&args),
    }
}
