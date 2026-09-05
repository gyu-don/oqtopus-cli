use std::fs::{self, Permissions};
use std::io::{BufRead, BufReader};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::ExitStatusExt;
use std::process::Stdio;

use crate::harness::{EnvironmentTemplate, TestContext};

#[test]
fn backend_info_outputs_metadata() {
    let context = TestContext::new();
    context.create_environment(
        EnvironmentTemplate::Backend,
        &[("engine_version", "v1.2.3")],
    );

    let output = context.run_snapshot_subject(["backend", "info"]);

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    // Pin the line-oriented key=value contract consumed by the Manager separately from formatting.
    let stdout = String::from_utf8(output.stdout.clone()).expect("metadata should be UTF-8");
    let fields: std::collections::HashMap<_, _> = stdout
        .lines()
        .map(|line| line.split_once('=').expect("each metadata row has a value"))
        .collect();
    assert_eq!(fields.get("template"), Some(&"backend"));
    assert_eq!(fields.get("environment_name"), Some(&"characterization"));
    assert_eq!(
        fields.get("environment_root").copied(),
        context.work_dir().to_str()
    );
    let install_root = context.root().join("xdg-data/oqtopus/backend/releases");
    assert_eq!(fields.get("install_root").copied(), install_root.to_str());
    assert_eq!(fields.get("engine_version"), Some(&"v1.2.3"));

    insta::assert_snapshot!("backend_info", context.render_output(&output));
}

// These cases exercise intentional Rust compatibility changes, so they always run Rust even
// when the characterization subject is Bash.
#[test]
fn backend_info_rejects_bare_template_key() {
    let context = TestContext::new();
    context.write_metadata(format!(
        "template\ninstall_root={}/releases\nenvironment_root={}\n",
        context.root().display(),
        context.work_dir().display()
    ));

    let output = context
        .rust_command(["backend", "info"])
        .output()
        .expect("run Rust CLI");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        output.stderr,
        b"Error: invalid .metadata: missing template.\n"
    );
}

#[test]
fn backend_info_accepts_crlf_and_preserves_output_bytes() {
    let context = TestContext::new();
    let metadata = format!(
        "template=backend\r\ninstall_root={}/releases\r\nenvironment_root={}\r\nengine_version=v1.2.3\r\n",
        context.root().display(),
        context.work_dir().display()
    );
    context.write_metadata(&metadata);

    let output = context
        .rust_command(["backend", "info"])
        .output()
        .expect("run Rust CLI");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_eq!(output.stdout, metadata.as_bytes());
    assert_eq!(
        fs::read(context.work_dir().join(".metadata")).expect("read metadata"),
        metadata.as_bytes()
    );
}

#[test]
fn backend_info_rejects_arguments() {
    let context = TestContext::new();

    let output = context.run_snapshot_subject(["backend", "info", "unexpected"]);

    insta::assert_snapshot!(
        "backend_info_extra_argument",
        context.render_output(&output)
    );
}

#[test]
fn backend_info_rejects_missing_metadata() {
    let context = TestContext::new();

    insta::assert_snapshot!(
        "backend_info_no_metadata",
        context.render_output(&context.run_snapshot_subject(["backend", "info"]))
    );
}

#[test]
fn backend_info_rejects_wrong_template() {
    let context = TestContext::new();
    context.create_environment(EnvironmentTemplate::Manager, &[]);

    insta::assert_snapshot!(
        "backend_info_wrong_template",
        context.render_output(&context.run_snapshot_subject(["backend", "info"]))
    );
}

#[test]
fn backend_info_rejects_missing_install_root() {
    let context = TestContext::new();
    context.write_metadata(format!(
        "template=backend\nenvironment_root={}\n",
        context.work_dir().display()
    ));

    insta::assert_snapshot!(
        "backend_info_missing_install_root",
        context.render_output(&context.run_snapshot_subject(["backend", "info"]))
    );
}

#[test]
fn backend_info_rejects_mismatched_environment_root() {
    let context = TestContext::new();
    context.write_metadata(format!(
        "template=backend\ninstall_root={}/releases\nenvironment_root={}/elsewhere\n",
        context.root().display(),
        context.root().display()
    ));

    insta::assert_snapshot!(
        "backend_info_mismatched_root",
        context.render_output(&context.run_snapshot_subject(["backend", "info"]))
    );
}

#[test]
fn backend_info_migrates_legacy_metadata_keys() {
    let context = TestContext::new();
    context.write_metadata(format!(
        "template=backend\ninstall_root={}/releases\nenv_name=legacy\nenv_root={}\nengine_version=v1.2.3\n",
        context.root().display(),
        context.work_dir().display()
    ));

    let output = context.run_snapshot_subject(["backend", "info"]);
    let migrated = fs::read(context.work_dir().join(".metadata")).expect("read migrated metadata");
    let expected = format!(
        "template=backend\ninstall_root={}/releases\nengine_version=v1.2.3\nenvironment_root={}\nenvironment_name=legacy\n",
        context.root().display(),
        context.work_dir().display()
    );

    // Check the persisted side effect directly; the snapshot independently covers stdout.
    assert_eq!(migrated, expected.as_bytes());
    insta::assert_snapshot!(
        "backend_info_legacy_metadata",
        context.render_output(&output)
    );
}

#[test]
fn backend_info_does_not_migrate_metadata_unwritable_by_an_unprivileged_owner() {
    // Exercise ordinary invocations with matching real/effective IDs. Root bypasses the file
    // permissions, and differing IDs are outside this fixture's scope.
    // SAFETY: these ID getters have no arguments or memory-safety preconditions.
    if unsafe {
        libc::getuid() == 0
            || libc::getuid() != libc::geteuid()
            || libc::getgid() != libc::getegid()
    } {
        return;
    }

    let context = TestContext::new();
    let original = format!(
        "template=backend\ninstall_root={}/releases\nenv_name=legacy\nenv_root={}\n",
        context.root().display(),
        context.work_dir().display()
    );
    let path = context.work_dir().join(".metadata");
    context.write_metadata(&original);
    // The owner may read but not write. Group/other write bits expose the difference between
    // `access(W_OK)` and merely checking whether any write bit is present.
    fs::set_permissions(&path, Permissions::from_mode(0o422))
        .expect("make metadata unwritable by its owner");

    let output = context.run_snapshot_subject(["backend", "info"]);

    assert!(output.status.success());
    assert_eq!(output.stdout, original.as_bytes());
    assert_eq!(
        fs::read(path).expect("read unchanged metadata"),
        original.as_bytes()
    );
}

#[test]
fn backend_info_exits_silently_on_broken_stdout_pipe() {
    let context = TestContext::new();
    let metadata = format!(
        "template=backend\ninstall_root={}/releases\nenvironment_root={}\npadding={}\n",
        context.root().display(),
        context.work_dir().display(),
        "x".repeat(8 * 1024 * 1024)
    );
    context.write_metadata(metadata);

    let mut child = context
        .rust_command(["backend", "info"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start Rust CLI");
    let stdout = child.stdout.take().expect("capture stdout");
    let mut reader = BufReader::new(stdout);
    let mut first_line = String::new();
    reader.read_line(&mut first_line).expect("read first line");
    assert_eq!(first_line, "template=backend\n");
    drop(reader);

    let output = child.wait_with_output().expect("wait for Rust CLI");
    assert_eq!(output.status.signal(), Some(libc::SIGPIPE));
    assert!(output.stderr.is_empty());
}
