//! Help texts, usage screens, completion scripts, and `version` output.

use crate::harness::*;

#[test]
fn top_level_help() {
    for (name, args) in [
        ("no_arguments", Vec::<&str>::new()),
        ("help", vec!["help"]),
        ("help_flag", vec!["--help"]),
    ] {
        let context = TestContext::new();
        snap!(format!("top_level_help__{name}"), context.run(args, 0));
    }
}

#[test]
fn command_help() {
    for (name, command) in [
        ("init", vec!["init"]),
        ("backend", vec!["backend"]),
        ("cloud_local", vec!["cloud-local"]),
        ("completion", vec!["completion"]),
        ("backend_install", vec!["backend", "install"]),
        ("backend_build", vec!["backend", "build"]),
        ("backend_versions", vec!["backend", "versions"]),
        ("backend_uninstall", vec!["backend", "uninstall"]),
        ("backend_update", vec!["backend", "update"]),
        ("backend_start", vec!["backend", "start"]),
        ("backend_stop", vec!["backend", "stop"]),
        ("backend_restart", vec!["backend", "restart"]),
        ("backend_device_status", vec!["backend", "device-status"]),
        ("manager", vec!["manager"]),
        ("manager_install", vec!["manager", "install"]),
        ("manager_versions", vec!["manager", "versions"]),
        ("manager_uninstall", vec!["manager", "uninstall"]),
        ("manager_update", vec!["manager", "update"]),
        ("manager_start", vec!["manager", "start"]),
        ("manager_stop", vec!["manager", "stop"]),
        ("manager_restart", vec!["manager", "restart"]),
        ("cloud_local_install", vec!["cloud-local", "install"]),
        ("cloud_local_versions", vec!["cloud-local", "versions"]),
        ("cloud_local_uninstall", vec!["cloud-local", "uninstall"]),
        ("cloud_local_update", vec!["cloud-local", "update"]),
        ("cloud_local_start", vec!["cloud-local", "start"]),
        ("cloud_local_stop", vec!["cloud-local", "stop"]),
        ("cloud_local_restart", vec!["cloud-local", "restart"]),
    ] {
        let display = command.join(" ");
        let help_context = TestContext::new();
        let mut help_args = command.clone();
        help_args.push("help");
        let help_output = help_context.run(help_args, 0);
        snap!(format!("command_help__{name}"), &help_output);

        let flag_context = TestContext::new();
        let mut flag_args = command;
        flag_args.push("--help");
        let flag_output = flag_context.run(flag_args, 0);
        assert_eq!(
            help_output, flag_output,
            "`{display}` help and --help must agree"
        );
    }
}

#[test]
fn dispatcher_without_arguments_prints_help() {
    for command in ["backend", "cloud-local", "manager"] {
        let help_context = TestContext::new();
        let help_output = help_context.run([command, "help"], 0);

        let bare_context = TestContext::new();
        let bare_output = bare_context.run([command], 0);
        assert_eq!(
            help_output, bare_output,
            "`{command}` with no arguments must match `{command} help`"
        );
    }
}

#[test]
fn version() {
    for (name, argument) in [("command", "version"), ("flag", "--version")] {
        let context = TestContext::new();
        let output = context.run([argument], 0);
        snap!(format!("version__{name}"), output);
    }
}

#[test]
fn completion_scripts() {
    for shell in ["bash", "zsh", "fish"] {
        let context = TestContext::new();
        snap!(
            format!("completion_scripts__{shell}"),
            context.run(["completion", shell], 0)
        );
    }
}
