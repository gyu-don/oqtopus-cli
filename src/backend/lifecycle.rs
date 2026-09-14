//! Backend service lifecycle commands.

use std::io::Write;

use crate::args::{is_help, single_target, validate_member};
use crate::environment::{Environment, validate_environment};
use crate::lifecycle::{LifecycleKind, LifecycleResult, result_code, success, usage};
use crate::metadata::metadata_get;
use crate::service::{BackgroundOutput, ServiceCommand, StopStyle, start_process, stop_process};
use crate::text::write_error;

use super::SERVICES;

// Startup follows dependency order; shutdown reverses it. Status uses SERVICES order.
const START_ORDER: [&str; 7] = [
    "gateway",
    "tranqu",
    "mitigator",
    "estimator",
    "combiner",
    "sse_engine",
    "core",
];
const STOP_ORDER: [&str; 7] = [
    "core",
    "sse_engine",
    "combiner",
    "estimator",
    "mitigator",
    "tranqu",
    "gateway",
];

pub(crate) fn backend_start<W: Write>(
    args: &[String],
    out: &mut W,
) -> Result<LifecycleResult, String> {
    if is_help(args) {
        return Ok(usage(LifecycleKind::BackendStart, 0));
    }
    let environment = validate_environment("backend")?;
    let Some(target) = args.first() else {
        return Ok(usage(LifecycleKind::BackendStart, 1));
    };
    let foreground = args.get(1).is_some_and(|arg| arg == "--foreground");
    if args.len() != 1 && !(args.len() == 2 && foreground) {
        return Ok(usage(LifecycleKind::BackendStart, 1));
    }
    if target == "all" {
        if foreground {
            return Err("oqtopus backend start all does not support --foreground. Start one service at a time in foreground mode.".into());
        }
        for service in START_ORDER {
            start_backend_service(&environment, service, false, out)?;
        }
    } else {
        let code = start_backend_service(&environment, target, foreground, out)?;
        return Ok(result_code(code));
    }
    Ok(success())
}

pub(crate) fn backend_stop<W: Write, E: Write>(
    args: &[String],
    out: &mut W,
    err: &mut E,
) -> Result<LifecycleResult, String> {
    if is_help(args) {
        return Ok(usage(LifecycleKind::BackendStop, 0));
    }
    let environment = validate_environment("backend")?;
    let Some(target) = single_target(args) else {
        return Ok(usage(LifecycleKind::BackendStop, 1));
    };
    if target == "all" {
        let failed = stop_many(&environment, &STOP_ORDER, out, err);
        return Ok(result_code(failed as i32));
    }
    validate_member(target, &SERVICES)?;
    stop_process(&environment.root, target, StopStyle::Backend, out)?;
    Ok(success())
}

pub(crate) fn backend_restart<W: Write, E: Write>(
    args: &[String],
    out: &mut W,
    err: &mut E,
) -> Result<LifecycleResult, String> {
    if is_help(args) {
        return Ok(usage(LifecycleKind::BackendRestart, 0));
    }
    let environment = validate_environment("backend")?;
    let Some(target) = single_target(args) else {
        return Ok(usage(LifecycleKind::BackendRestart, 1));
    };
    if target == "all" {
        if stop_many(&environment, &STOP_ORDER, out, err) {
            return Ok(result_code(1));
        }
        for service in START_ORDER {
            start_backend_service(&environment, service, false, out)?;
        }
    } else {
        validate_member(target, &SERVICES)?;
        stop_process(&environment.root, target, StopStyle::Backend, out)?;
        start_backend_service(&environment, target, false, out)?;
    }
    Ok(success())
}

fn start_backend_service<W: Write>(
    environment: &Environment,
    service: &str,
    foreground: bool,
    out: &mut W,
) -> Result<i32, String> {
    validate_member(service, &SERVICES)?;
    start_process(
        &environment.root,
        service,
        || backend_command(environment, service),
        foreground,
        BackgroundOutput::Null,
        false,
        out,
    )
}

fn backend_command(environment: &Environment, service: &str) -> Result<ServiceCommand, String> {
    let (component, project_name, module) = match service {
        "core" | "sse_engine" => ("engine", Some("core"), "oqtopus_engine_core.app"),
        "mitigator" => ("engine", Some("mitigator"), "oqtopus_engine_mitigator.app"),
        "estimator" => ("engine", Some("estimator"), "oqtopus_engine_estimator.app"),
        "combiner" => ("engine", Some("combiner"), "oqtopus_engine_combiner.app"),
        "tranqu" => ("tranqu", None, "tranqu_server.proto.service"),
        "gateway" => ("gateway", None, "device_gateway.service"),
        _ => return Err(format!("unknown service: {service}")),
    };
    let metadata = environment.metadata.as_str();
    let version = metadata_get(metadata, &format!("{component}_version")).ok_or_else(|| {
        format!("cannot start '{service}'. Missing {component}_version in .metadata.")
    })?;
    let mut project = if version.starts_with("branch:") {
        environment.root.join(component)
    } else {
        environment
            .install_root
            .join(format!("{component}-{version}"))
    };
    if let Some(name) = project_name {
        project.push(name);
    }
    if !project.is_dir() {
        return Err(format!(
            "cannot start '{service}'. Installed release directory not found: {}",
            project.display()
        ));
    }
    Ok(ServiceCommand::uv(vec![
        "run".into(),
        "--project".into(),
        project.display().to_string(),
        "python".into(),
        "-m".into(),
        module.into(),
        "-c".into(),
        environment
            .root
            .join(format!("config/{service}/config.yaml"))
            .display()
            .to_string(),
        "-l".into(),
        environment
            .root
            .join(format!("config/{service}/logging.yaml"))
            .display()
            .to_string(),
    ]))
}

fn stop_many<W: Write, E: Write>(
    environment: &Environment,
    services: &[&str],
    out: &mut W,
    err: &mut E,
) -> bool {
    let mut failed = false;
    for service in services {
        if let Err(error) = stop_process(&environment.root, service, StopStyle::Backend, out) {
            let _ = write_error(err, &error);
            failed = true;
        }
    }
    failed
}
