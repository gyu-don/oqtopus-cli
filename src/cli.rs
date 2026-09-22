//! Command routing and output orchestration during the incremental Rust migration.

use std::io;

use crate::backend::{
    backend_build, backend_device_status, backend_info, backend_install, backend_restart,
    backend_start, backend_status, backend_stop, backend_uninstall, backend_update,
    backend_versions,
};
use crate::cloud_local::{
    cloud_local_info, cloud_local_install, cloud_local_restart, cloud_local_start,
    cloud_local_status, cloud_local_stop, cloud_local_uninstall, cloud_local_update,
    cloud_local_versions,
};
use crate::init::init;
use crate::legacy::run_legacy;
use crate::manager::{
    manager_info, manager_install, manager_restart, manager_start, manager_status, manager_stop,
    manager_uninstall, manager_update, manager_versions,
};
use crate::text;
use crate::version::version_info;

const EXIT_SUCCESS: i32 = 0;
const EXIT_FAILURE: i32 = 1;

/// Implementation selected for a command-line invocation.
///
/// Routing is intentionally coarse while the Rust migration is in progress: anything not listed
/// here remains the legacy CLI's responsibility.
pub(crate) enum Route {
    Help,
    Version,
    Init,
    BackendInfo,
    BackendStatus,
    BackendDeviceStatus,
    BackendVersions,
    BackendInstall,
    BackendBuild,
    BackendUninstall,
    BackendUpdate,
    BackendStart,
    BackendStop,
    BackendRestart,
    CloudLocalInfo,
    CloudLocalStatus,
    CloudLocalVersions,
    CloudLocalInstall,
    CloudLocalUninstall,
    CloudLocalUpdate,
    CloudLocalStart,
    CloudLocalStop,
    CloudLocalRestart,
    ManagerInfo,
    ManagerStatus,
    ManagerVersions,
    ManagerInstall,
    ManagerUninstall,
    ManagerUpdate,
    ManagerStart,
    ManagerStop,
    ManagerRestart,
    Legacy,
}

/// Selects the Rust implementation for migrated commands and [`Route::Legacy`] otherwise.
pub(crate) fn route(args: &[String]) -> Route {
    // Only the two leading words select a route; the rest belongs to the command itself.
    let command = args.first().map(String::as_str);
    let action = args.get(1).map(String::as_str);

    match (command, action) {
        (None, _) | (Some("help" | "--help"), _) => Route::Help,
        (Some("version" | "--version"), _) => Route::Version,
        (Some("init"), _) => Route::Init,
        (Some("backend"), Some("info")) => Route::BackendInfo,
        (Some("backend"), Some("status")) => Route::BackendStatus,
        (Some("backend"), Some("device-status")) => Route::BackendDeviceStatus,
        (Some("backend"), Some("versions")) => Route::BackendVersions,
        (Some("backend"), Some("install")) => Route::BackendInstall,
        (Some("backend"), Some("build")) => Route::BackendBuild,
        (Some("backend"), Some("uninstall")) => Route::BackendUninstall,
        (Some("backend"), Some("update")) => Route::BackendUpdate,
        (Some("backend"), Some("start")) => Route::BackendStart,
        (Some("backend"), Some("stop")) => Route::BackendStop,
        (Some("backend"), Some("restart")) => Route::BackendRestart,
        (Some("cloud-local"), Some("info")) => Route::CloudLocalInfo,
        (Some("cloud-local"), Some("status")) => Route::CloudLocalStatus,
        (Some("cloud-local"), Some("versions")) => Route::CloudLocalVersions,
        (Some("cloud-local"), Some("install")) => Route::CloudLocalInstall,
        (Some("cloud-local"), Some("uninstall")) => Route::CloudLocalUninstall,
        (Some("cloud-local"), Some("update")) => Route::CloudLocalUpdate,
        (Some("cloud-local"), Some("start")) => Route::CloudLocalStart,
        (Some("cloud-local"), Some("stop")) => Route::CloudLocalStop,
        (Some("cloud-local"), Some("restart")) => Route::CloudLocalRestart,
        (Some("manager"), Some("info")) => Route::ManagerInfo,
        (Some("manager"), Some("status")) => Route::ManagerStatus,
        (Some("manager"), Some("versions")) => Route::ManagerVersions,
        (Some("manager"), Some("install")) => Route::ManagerInstall,
        (Some("manager"), Some("uninstall")) => Route::ManagerUninstall,
        (Some("manager"), Some("update")) => Route::ManagerUpdate,
        (Some("manager"), Some("start")) => Route::ManagerStart,
        (Some("manager"), Some("stop")) => Route::ManagerStop,
        (Some("manager"), Some("restart")) => Route::ManagerRestart,
        _ => Route::Legacy,
    }
}

/// Executes one CLI invocation and returns its process exit status.
pub(crate) fn run(args: &[String]) -> i32 {
    // Template subcommands consume two leading words. `init` consumes only its top-level word.
    let command_args = args.get(2..).unwrap_or_default();
    let init_args = args.get(1..).unwrap_or_default();

    // Commands report their own exit status; a returned message is always a failure.
    //
    // Most commands compute a result and then render it, so they take the stdout lock only for the
    // rendering step. The install, build, uninstall, and update routes instead stream progress
    // while they work, so they hold one lock across both the command and its final output; that
    // keeps progress and result in a single ordered stream.
    let outcome: Result<i32, String> = match route(args) {
        Route::Help => text::write_help(&mut io::stdout().lock())
            .map(|()| EXIT_SUCCESS)
            .map_err(|error| format!("failed to write help: {error}")),
        Route::Version => text::write_version(&mut io::stdout().lock(), &version_info())
            .map(|()| EXIT_SUCCESS)
            .map_err(|error| format!("failed to write version: {error}")),
        Route::Init => init(init_args).and_then(|result| {
            text::write_init(&mut io::stdout().lock(), &result)
                .map(|()| result.exit_code())
                .map_err(|error| format!("failed to write init result: {error}"))
        }),
        Route::BackendInfo => backend_info(command_args).and_then(|info| {
            text::write_backend_info(&mut io::stdout().lock(), &info)
                .map(|()| EXIT_SUCCESS)
                .map_err(|error| format!("failed to write backend info: {error}"))
        }),
        Route::BackendStatus => backend_status(command_args).and_then(|status| {
            text::write_backend_status(&mut io::stdout().lock(), &status)
                .map(|()| EXIT_SUCCESS)
                .map_err(|error| format!("failed to write backend status: {error}"))
        }),
        Route::BackendDeviceStatus => backend_device_status(command_args).and_then(|status| {
            text::write_backend_device_status(&mut io::stdout().lock(), &status)
                .map(|()| status.exit_code())
                .map_err(|error| format!("failed to write backend device status: {error}"))
        }),
        Route::BackendVersions => backend_versions(command_args).and_then(|result| {
            text::write_versions(&mut io::stdout().lock(), &result)
                .map(|()| result.exit_code())
                .map_err(|error| format!("failed to write backend versions: {error}"))
        }),
        Route::BackendInstall => operation_stdout(
            |out| backend_install(command_args, out),
            "failed to write backend install result",
        ),
        Route::BackendBuild => operation_stdout(
            |out| backend_build(command_args, out),
            "failed to write backend build result",
        ),
        Route::BackendUninstall => operation_stdout(
            |out| backend_uninstall(command_args, out),
            "failed to write backend uninstall result",
        ),
        Route::BackendUpdate => operation_stdout(
            |out| backend_update(command_args, out),
            "failed to write backend update result",
        ),
        Route::BackendStart => lifecycle_stdout(|out, _err| backend_start(command_args, out)),
        Route::BackendStop => lifecycle_stdout(|out, err| backend_stop(command_args, out, err)),
        Route::BackendRestart => {
            lifecycle_stdout(|out, err| backend_restart(command_args, out, err))
        }
        Route::CloudLocalInfo => cloud_local_info(command_args).and_then(|info| {
            text::write_cloud_local_info(&mut io::stdout().lock(), &info)
                .map(|()| EXIT_SUCCESS)
                .map_err(|error| format!("failed to write cloud-local info: {error}"))
        }),
        Route::CloudLocalStatus => cloud_local_status(command_args).and_then(|status| {
            text::write_cloud_local_status(&mut io::stdout().lock(), &status)
                .map(|()| EXIT_SUCCESS)
                .map_err(|error| format!("failed to write cloud-local status: {error}"))
        }),
        Route::CloudLocalVersions => cloud_local_versions(command_args).and_then(|result| {
            text::write_versions(&mut io::stdout().lock(), &result)
                .map(|()| result.exit_code())
                .map_err(|error| format!("failed to write cloud-local versions: {error}"))
        }),
        Route::CloudLocalInstall => operation_stdout(
            |out| cloud_local_install(command_args, out),
            "failed to write cloud-local install result",
        ),
        Route::CloudLocalUninstall => operation_stdout(
            |out| cloud_local_uninstall(command_args, out),
            "failed to write cloud-local uninstall result",
        ),
        Route::CloudLocalUpdate => operation_stdout(
            |out| cloud_local_update(command_args, out),
            "failed to write cloud-local update result",
        ),
        Route::CloudLocalStart => {
            lifecycle_stdout(|out, err| cloud_local_start(command_args, out, err))
        }
        Route::CloudLocalStop => {
            lifecycle_stdout(|out, err| cloud_local_stop(command_args, out, err))
        }
        Route::CloudLocalRestart => {
            lifecycle_stdout(|out, err| cloud_local_restart(command_args, out, err))
        }
        Route::ManagerInfo => manager_info(command_args).and_then(|info| {
            text::write_manager_info(&mut io::stdout().lock(), &info)
                .map(|()| EXIT_SUCCESS)
                .map_err(|error| format!("failed to write manager info: {error}"))
        }),
        Route::ManagerStatus => manager_status(command_args).and_then(|status| {
            text::write_manager_status(&mut io::stdout().lock(), &status)
                .map(|()| EXIT_SUCCESS)
                .map_err(|error| format!("failed to write manager status: {error}"))
        }),
        Route::ManagerVersions => manager_versions(command_args).and_then(|result| {
            text::write_versions(&mut io::stdout().lock(), &result)
                .map(|()| result.exit_code())
                .map_err(|error| format!("failed to write manager versions: {error}"))
        }),
        Route::ManagerInstall => operation_stdout(
            |out| manager_install(command_args, out),
            "failed to write manager install result",
        ),
        Route::ManagerUninstall => operation_stdout(
            |out| manager_uninstall(command_args, out),
            "failed to write manager uninstall result",
        ),
        Route::ManagerUpdate => operation_stdout(
            |out| manager_update(command_args, out),
            "failed to write manager update result",
        ),
        Route::ManagerStart => lifecycle_stdout(|out, _err| manager_start(command_args, out)),
        Route::ManagerStop => lifecycle_stdout(|out, _err| manager_stop(command_args, out)),
        Route::ManagerRestart => lifecycle_stdout(|out, _err| manager_restart(command_args, out)),
        Route::Legacy => run_legacy(args),
    };

    match outcome {
        Ok(code) => code,
        Err(error) => {
            let _ = text::write_error(&mut io::stderr().lock(), &error);
            EXIT_FAILURE
        }
    }
}

fn lifecycle_stdout(
    command: impl FnOnce(
        &mut io::StdoutLock<'_>,
        &mut io::StderrLock<'_>,
    ) -> Result<crate::lifecycle::LifecycleOutcome, String>,
) -> Result<i32, String> {
    let mut stdout = io::stdout().lock();
    let mut stderr = io::stderr().lock();
    command(&mut stdout, &mut stderr).and_then(|result| {
        text::write_lifecycle(&mut stdout, &result)
            .map(|()| result.exit_code())
            .map_err(|error| format!("failed to write lifecycle result: {error}"))
    })
}

fn operation_stdout(
    command: impl FnOnce(&mut io::StdoutLock<'_>) -> Result<crate::operations::OperationOutcome, String>,
    write_error_context: &str,
) -> Result<i32, String> {
    let mut stdout = io::stdout().lock();
    command(&mut stdout).and_then(|result| {
        text::write_operation(&mut stdout, &result)
            .map(|()| result.exit_code())
            .map_err(|error| format!("{write_error_context}: {error}"))
    })
}
