//! The argument-error contract: unknown commands, malformed invocations,
//! and missing external dependencies.

use std::fs;

use crate::harness::*;

#[test]
fn representative_errors() {
    for (name, args) in [
        ("unknown_top_level_command", vec!["unknown"]),
        ("unknown_backend_command", vec!["backend", "unknown"]),
        (
            "unknown_cloud_local_command",
            vec!["cloud-local", "unknown"],
        ),
        ("unknown_manager_command", vec!["manager", "unknown"]),
        (
            "unsupported_completion_shell",
            vec!["completion", "powershell"],
        ),
        (
            "backend_status_outside_environment",
            vec!["backend", "status"],
        ),
        ("init_no_arguments", vec!["init"]),
        ("init_missing_template_flag", vec!["init", "demo"]),
        (
            "init_template_flag_without_value",
            vec!["init", "demo", "--template"],
        ),
        (
            "init_extra_argument",
            vec!["init", "demo", "--template", "backend", "extra"],
        ),
        (
            "init_branch_flag_without_value",
            vec!["init", "demo", "--template", "backend", "--branch"],
        ),
        (
            "manager_status_outside_environment",
            vec!["manager", "status"],
        ),
        (
            "manager_status_extra_argument",
            vec!["manager", "status", "extra"],
        ),
        (
            "manager_info_extra_argument",
            vec!["manager", "info", "extra"],
        ),
        (
            "manager_versions_extra_argument",
            vec!["manager", "versions", "extra"],
        ),
        ("backend_versions_no_arguments", vec!["backend", "versions"]),
        (
            "backend_versions_extra_argument",
            vec!["backend", "versions", "engine", "extra"],
        ),
        ("completion_no_arguments", vec!["completion"]),
        (
            "completion_extra_argument",
            vec!["completion", "bash", "zsh"],
        ),
        (
            "backend_status_extra_argument",
            vec!["backend", "status", "extra"],
        ),
        (
            "backend_info_extra_argument",
            vec!["backend", "info", "extra"],
        ),
    ] {
        let context = TestContext::new();
        snap!(
            format!("representative_errors__{name}"),
            context.run(args, 1)
        );
    }
}

#[test]
fn need_command_reports_missing_dependencies() {
    // `curl` itself is absent from PATH.
    {
        let context = TestContext::new();
        let empty_path = context.root().join("no-tools");
        fs::create_dir_all(&empty_path).expect("create empty PATH directory");

        let output = context.run_with_env(
            ["backend", "versions", "engine"],
            1,
            [("PATH", empty_path.as_os_str())],
        );

        snap!("need_command__curl_missing", output);
    }

    // `curl` is present (the fake tool) but `jq` is not.
    {
        let context = TestContext::new();

        let output = context.run_with_env(
            ["backend", "versions", "engine"],
            1,
            [("PATH", context.fake_tools.bin_dir().as_os_str())],
        );

        snap!("need_command__jq_missing", output);
    }
}
