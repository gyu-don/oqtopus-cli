use std::process::{Command, Output};

const FORBID_LEGACY_FALLBACK: &str = "OQTOPUS_FORBID_LEGACY_FALLBACK";

fn run_rust(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_oqtopus"))
        .args(args)
        .env(FORBID_LEGACY_FALLBACK, "1")
        .output()
        .expect("Rust CLI should run")
}

#[test]
fn version_uses_the_compiled_package_version() {
    let expected = format!("oqtopus {}\n", env!("CARGO_PKG_VERSION"));

    for args in [
        &["version"][..],
        &["--version"],
        &["version", "ignored"],
        &["--version", "ignored"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_oqtopus"))
            .args(args)
            .env(FORBID_LEGACY_FALLBACK, "1")
            .env("OQTOPUS_CLI_VERSION", "must-not-be-used")
            .output()
            .expect("Rust CLI should run");

        assert!(
            output.status.success(),
            "Rust failed for arguments {args:?}"
        );
        assert!(output.stderr.is_empty(), "Rust wrote stderr for {args:?}");
        assert_eq!(
            output.stdout,
            expected.as_bytes(),
            "wrong version for {args:?}"
        );
    }
}

#[test]
fn unmigrated_route_honors_fallback_forbidden() {
    let output = run_rust(&["not-yet-migrated"]);

    assert_eq!(output.status.code(), Some(125));
    assert!(output.stdout.is_empty());
    assert_eq!(
        output.stderr,
        b"Error: legacy Bash fallback is forbidden for this invocation.\n"
    );
}
