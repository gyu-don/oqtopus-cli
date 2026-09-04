use std::env;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{self, Command};

fn main() {
    let legacy_cli = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("bin/oqtopus");

    let error = Command::new("bash")
        .arg(legacy_cli)
        .args(env::args_os().skip(1))
        .exec();

    eprintln!("Error: failed to run the legacy OQTOPUS CLI: {error}");
    process::exit(126);
}
