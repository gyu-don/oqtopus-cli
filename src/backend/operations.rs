//! Backend component operation entrypoints and declarations.

use std::io::Write;

use crate::args::is_help;
use crate::environment::validate_environment;
use crate::metadata::metadata_get;
use crate::operations::{
    ComponentKind, OperationKind, OperationResult, build_sse_runtime, component_complete,
    find_component, install_release, install_version, success, uninstall, usage,
};
use crate::progress::Reporter;

use super::components::COMPONENTS;

pub(crate) fn backend_install<W: Write>(
    args: &[String],
    out: &mut W,
) -> Result<OperationResult, String> {
    if is_help(args) {
        return Ok(usage(OperationKind::BackendInstall, 0));
    }
    let environment = validate_environment("backend")?;
    let Some(component_name) = args.first() else {
        return Ok(usage(OperationKind::BackendInstall, 1));
    };
    let mut version = "";
    let mut skip_sse_build = false;
    for arg in &args[1..] {
        if arg == "--skip-sse-build" {
            skip_sse_build = true;
        } else if arg.starts_with('-') {
            return Err(format!("unknown install option: {arg}"));
        } else if !version.is_empty() {
            return Ok(usage(OperationKind::BackendInstall, 1));
        } else {
            version = arg;
        }
    }

    let mut reporter = Reporter::new(out);
    if component_name == "all" {
        if !version.is_empty() {
            return Err("oqtopus backend install all does not accept a version argument.".into());
        }
        for component in COMPONENTS {
            install_release(&environment, component, None, skip_sse_build, &mut reporter)?;
        }
        return Ok(success());
    }
    if skip_sse_build && component_name != "engine" {
        return Err("--skip-sse-build is only supported for 'engine' and 'all'.".into());
    }
    let component = find_component(&COMPONENTS, component_name)?;
    install_version(
        &environment,
        component,
        (!version.is_empty()).then_some(version),
        skip_sse_build,
        &mut reporter,
    )?;
    Ok(success())
}

pub(crate) fn backend_uninstall<W: Write>(
    args: &[String],
    out: &mut W,
) -> Result<OperationResult, String> {
    if is_help(args) {
        return Ok(usage(OperationKind::BackendUninstall, 0));
    }
    let environment = validate_environment("backend")?;
    if args.len() != 2 || args[0].is_empty() || args[1].is_empty() {
        return Ok(usage(OperationKind::BackendUninstall, 1));
    }
    let component = find_component(&COMPONENTS, &args[0])?;
    let mut reporter = Reporter::new(out);
    uninstall(&environment, component, &args[1], &mut reporter)?;
    Ok(success())
}

pub(crate) fn backend_update<W: Write>(
    args: &[String],
    out: &mut W,
) -> Result<OperationResult, String> {
    if is_help(args) {
        return Ok(usage(OperationKind::BackendUpdate, 0));
    }
    let environment = validate_environment("backend")?;
    if args.len() != 1 || args[0].is_empty() {
        return Ok(usage(OperationKind::BackendUpdate, 1));
    }
    let component = find_component(&COMPONENTS, &args[0])?;
    install_release(
        &environment,
        component,
        None,
        false,
        &mut Reporter::new(out),
    )?;
    Ok(success())
}

pub(crate) fn backend_build<W: Write>(
    args: &[String],
    out: &mut W,
) -> Result<OperationResult, String> {
    if is_help(args) {
        return Ok(usage(OperationKind::BackendBuild, 0));
    }
    let environment = validate_environment("backend")?;
    if args.len() != 1 || args[0] != "sse-runtime" {
        return Ok(usage(OperationKind::BackendBuild, 1));
    }
    let metadata = environment.metadata.as_str();
    let version = metadata_get(metadata, "engine_version")
        .ok_or("engine is not installed in this backend environment.")?;
    let target = if version.starts_with("branch:") {
        environment.root.join("engine")
    } else {
        environment.install_root.join(format!("engine-{version}"))
    };
    if !target.is_dir() {
        return Err(format!(
            "installed engine release not found: {}",
            target.display()
        ));
    }
    if !component_complete(ComponentKind::Engine, &target) {
        return Err(format!(
            "engine {version} is not completely installed in this backend environment."
        ));
    }
    build_sse_runtime(&environment, &target, &mut Reporter::new(out))?;
    Ok(success())
}
