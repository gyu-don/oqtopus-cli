//! `oqtopus backend`: environment validation, info, branch install/uninstall,
//! and the sse-runtime build.

use std::fs;

use crate::harness::*;

#[test]
fn backend_info_reports_metadata_verbatim() {
    let context = TestContext::new();
    EnvFixture::backend(&context, &[]);

    let output = context.run(["backend", "info"], 0);

    snap!("backend_info__metadata", output);
}

#[test]
fn backend_environment_validation_errors() {
    // No `.metadata` at all.
    {
        let context = TestContext::new();
        let output = context.run(["backend", "info"], 1);
        snap!("backend_env__no_metadata", output);
    }

    // `.metadata` exists but belongs to a different template.
    {
        let context = TestContext::new();
        let work = context.work_dir();
        fs::create_dir_all(&work).expect("create work dir");
        fs::write(
            work.join(".metadata"),
            format!(
                "template=cloud-local\nenvironment_root={}\ninstall_root={}\n",
                work.display(),
                context
                    .root()
                    .join("xdg-data/oqtopus/backend/releases")
                    .display(),
            ),
        )
        .expect("write .metadata");
        let output = context.run(["backend", "info"], 1);
        snap!("backend_env__wrong_template", output);
    }

    // `template=backend` and a correct `environment_root`, but no `install_root`.
    {
        let context = TestContext::new();
        let work = context.work_dir();
        fs::create_dir_all(&work).expect("create work dir");
        fs::write(
            work.join(".metadata"),
            format!("template=backend\nenvironment_root={}\n", work.display()),
        )
        .expect("write .metadata");
        let output = context.run(["backend", "info"], 1);
        snap!("backend_env__missing_install_root", output);
    }

    // `environment_root` in `.metadata` does not match the current directory.
    {
        let context = TestContext::new();
        let work = context.work_dir();
        fs::create_dir_all(&work).expect("create work dir");
        let mismatched_env_root = context.root().join("elsewhere");
        fs::write(
            work.join(".metadata"),
            format!(
                "template=backend\ninstall_root={}\nenvironment_root={}\n",
                context
                    .root()
                    .join("xdg-data/oqtopus/backend/releases")
                    .display(),
                mismatched_env_root.display(),
            ),
        )
        .expect("write .metadata");
        let output = context.run(["backend", "info"], 1);
        snap!("backend_env__env_root_mismatch", output);
    }
}

#[test]
fn backend_install_branch_writes_metadata_binding() {
    let context = TestContext::new();
    EnvFixture::backend(&context, &[]);
    // A branch install makes two curl calls: first the ref advertisement that
    // resolves the branch to a commit id, then the codeload tarball for that
    // commit (whose single top-level directory gets stripped on extraction).
    context
        .fake_tools
        .fixture_call("curl", 1)
        .stdout(advertised_refs(&[]));
    let checkout = build_targz(context.root(), "tranqu-server-checkout", |contents| {
        fs::write(
            contents.join("pyproject.toml"),
            "[project]\nname = \"tranqu\"\n",
        )
        .expect("write tranqu pyproject fixture");
    });
    context.fake_tools.fixture_call("curl", 2).stdout(checkout);
    context.fake_tools.fixture("uv");

    let output = context.run(["backend", "install", "tranqu", "branch:main"], 0);
    assert_eq!(context.fake_tools.call_count("curl"), 2);

    if context.invoke_with_bash {
        snap!(
            "bash_external_calls__backend_install_branch",
            normalize(&context.fake_tools.log(), context.root())
        );
    }

    snap!(
        "backend_install__branch_binding",
        render_observation(&context, &output, &[".metadata"])
    );
}

#[test]
fn backend_uninstall_branch_clears_metadata_binding() {
    let context = TestContext::new();
    EnvFixture::backend(&context, &[("tranqu_version", "branch:main")]);
    fs::create_dir_all(context.work_dir().join("tranqu")).expect("create tranqu branch checkout");

    let output = context.run(["backend", "uninstall", "tranqu", "branch:main"], 0);

    snap!(
        "backend_uninstall__branch_binding",
        render_observation(&context, &output, &[".metadata"])
    );
}

/// Lays out a backend environment with an installed `engine` release ready
/// for `backend build sse-runtime`, and writes `config/.env` with `contents`.
fn setup_sse_runtime_build(context: &TestContext, env_dot_env_contents: &str) {
    let env = EnvFixture::backend(context, &[("engine_version", "v1.2.3")]);
    let release = env.install_release("engine-v1.2.3");
    for project in ["core", "combiner", "estimator", "mitigator"] {
        fs::create_dir_all(release.join(project).join(".venv"))
            .unwrap_or_else(|error| panic!("create {project}/.venv: {error}"));
    }
    fs::create_dir_all(release.join("sse_runtime")).expect("create sse_runtime dir");
    fs::write(
        release.join("sse_runtime").join("Dockerfile"),
        "FROM scratch\n",
    )
    .expect("write sse_runtime Dockerfile");

    let config_dir = context.work_dir().join("config");
    fs::create_dir_all(&config_dir).expect("create config dir");
    fs::write(config_dir.join(".env"), env_dot_env_contents).expect("write config/.env");

    context.fake_tools.fixture("docker");
}

#[test]
fn sse_runtime_build_reads_the_env_config() {
    let success_cases = [
        ("double_quoted", "SSE_CONTAINER_IMAGE=\"img:1\"\n"),
        ("single_quoted", "SSE_CONTAINER_IMAGE='img:2'\n"),
        ("value_contains_separator", "SSE_CONTAINER_IMAGE=a=b=c\n"),
        (
            "after_comments",
            "# SSE_CONTAINER_IMAGE=ignored\n   # indented comment\n\nSSE_CONTAINER_IMAGE=img:5\n",
        ),
        ("empty_value", "SSE_CONTAINER_IMAGE=\n"),
    ];
    for (name, contents) in success_cases {
        let context = TestContext::new();
        setup_sse_runtime_build(&context, contents);

        let output = context.run(["backend", "build", "sse-runtime"], 0);
        assert!(output.starts_with("exit: 0\n"), "{output}");

        if context.invoke_with_bash {
            snap!(
                format!("sse_build__{name}"),
                normalize(&context.fake_tools.log(), context.root())
            );
        }
    }

    let error_cases = [
        ("key_missing", "OTHER=x\n"),
        ("key_indented", "   SSE_CONTAINER_IMAGE=img:4\n"),
    ];
    for (name, contents) in error_cases {
        let context = TestContext::new();
        setup_sse_runtime_build(&context, contents);

        let output = context.run(["backend", "build", "sse-runtime"], 1);

        snap!(format!("sse_build__{name}"), output);
    }
}
