//! `oqtopus init`: environment-name validation, template rendering for the
//! backend and manager templates, and the `--branch` flag.

use std::fs;
use std::path::Path;

use crate::harness::*;

#[test]
fn init_rejects_invalid_requests_without_side_effects() {
    for (name, args) in [
        (
            "init_rejects__invalid_env_name",
            vec!["init", "Bad_Name", "--template", "backend"],
        ),
        (
            "init_rejects__unknown_template",
            vec!["init", "demo", "--template", "nonexistent"],
        ),
    ] {
        let context = TestContext::new();
        let output = context.run(args, 1);
        snap!(name, render_observation(&context, &output, &[]));
    }
}

#[test]
fn init_backend_creates_rendered_environment() {
    let context = TestContext::new();
    let archive = build_backend_template_archive(context.root());
    context.fake_tools.fixture("curl").stdout(&archive);
    context
        .fake_tools
        .fixture("date")
        .stdout("2031-12-13T14:15:16Z\n");

    let output = context.run(["init", "demo", "--template", "backend"], 0);

    if context.invoke_with_bash {
        assert_eq!(context.fake_tools.call_count("date"), 1);
        snap!(
            "bash_external_calls__init_backend",
            normalize(&context.fake_tools.log(), context.root())
        );
    }

    snap!(
        "init_backend_creates_rendered_environment",
        render_observation(
            &context,
            &output,
            &[
                "demo/.metadata",
                "demo/config/.env",
                "demo/config/nested/backend.toml",
                "demo/compose.yaml",
            ],
        )
    );
}

#[test]
fn init_validates_env_name_boundaries() {
    for (snapshot_name, env_name) in [
        ("env_name__leading_hyphen", "-lead"),
        ("env_name__leading_dot", ".lead"),
        ("env_name__leading_underscore", "_lead"),
        ("env_name__uppercase", "UPPER"),
        ("env_name__space", "a b"),
        ("env_name__slash", "a/b"),
        ("env_name__single_digit", "0"),
        ("env_name__punctuation_tail", "a.b-c_d"),
    ] {
        let context = TestContext::new();
        let output = context.run(["init", env_name, "--template", "nonexistent"], 1);
        snap!(snapshot_name, output);
    }
}

#[test]
fn init_branch_flag_downloads_the_template_from_that_branch() {
    let context = TestContext::new();
    let archive = build_backend_template_archive(context.root());
    context.fake_tools.fixture("curl").stdout(&archive);
    context
        .fake_tools
        .fixture("date")
        .stdout("2031-12-13T14:15:16Z\n");

    let output = context.run(
        [
            "init",
            "demo",
            "--template",
            "backend",
            "--branch",
            "feature/x",
        ],
        0,
    );

    if context.invoke_with_bash {
        // The recorded URL is the observable effect of --branch.
        snap!(
            "bash_external_calls__init_backend_branch",
            normalize(&context.fake_tools.log(), context.root())
        );
    }

    snap!("init_backend__branch_flag", output);
}

fn build_manager_template_archive(root: &Path) -> Vec<u8> {
    build_targz(root, "oqtopus-cli-main", |contents| {
        let template = contents.join("templates/manager");
        fs::create_dir_all(template.join("config"))
            .expect("create manager template fixture directories");
        fs::write(
            template.join("config/config.yaml"),
            "manager:\n  port: 8080\n",
        )
        .expect("write manager config fixture");
        fs::write(template.join("config/logging.yaml"), "version: 1\n")
            .expect("write manager logging fixture");
    })
}

#[test]
fn init_manager_creates_environment() {
    let context = TestContext::new();
    let archive = build_manager_template_archive(context.root());
    context.fake_tools.fixture("curl").stdout(&archive);
    context
        .fake_tools
        .fixture("date")
        .stdout("2031-12-13T14:15:16Z\n");

    let output = context.run(["init", "demo", "--template", "manager"], 0);

    snap!(
        "init_manager_creates_environment",
        render_observation(
            &context,
            &output,
            &["demo/.metadata", "demo/config/config.yaml"],
        )
    );
}
