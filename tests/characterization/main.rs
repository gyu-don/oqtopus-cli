use std::path::PathBuf;
use std::process::{Command, Output};

const FORBID_LEGACY_FALLBACK: &str = "OQTOPUS_FORBID_LEGACY_FALLBACK";
const CHARACTERIZATION_SOURCE: &str = "OQTOPUS_CHARACTERIZATION_SOURCE";

fn run_bash(args: &[&str]) -> Output {
    let legacy_cli = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("bin/oqtopus");

    Command::new("bash")
        .arg(legacy_cli)
        .args(args)
        .output()
        .expect("legacy Bash CLI should run")
}

fn run_rust(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_oqtopus"))
        .args(args)
        .env(FORBID_LEGACY_FALLBACK, "1")
        .output()
        .expect("Rust CLI should run")
}

fn run_snapshot_subject(args: &[&str]) -> Output {
    match std::env::var(CHARACTERIZATION_SOURCE).as_deref() {
        Err(std::env::VarError::NotPresent) => run_rust(args),
        Ok("bash") => run_bash(args),
        Ok(source) => panic!("unsupported {CHARACTERIZATION_SOURCE} value: {source}"),
        Err(error) => panic!("invalid {CHARACTERIZATION_SOURCE} value: {error}"),
    }
}

#[test]
fn top_level_help() {
    let canonical = run_snapshot_subject(&[]);

    assert!(canonical.status.success());
    assert!(canonical.stderr.is_empty());

    let stdout = String::from_utf8(canonical.stdout.clone()).expect("help should be UTF-8");
    insta::assert_snapshot!(stdout);

    for args in [
        &[][..],
        &["help"][..],
        &["--help"],
        &["help", "ignored"],
        &["--help", "ignored"],
    ] {
        let alias = run_snapshot_subject(args);

        assert!(alias.status.success(), "CLI failed for arguments {args:?}");
        assert!(alias.stderr.is_empty(), "CLI wrote stderr for {args:?}");
        assert_eq!(alias.stdout, canonical.stdout, "help differed for {args:?}");
    }
}
