//! `oqtopus manager`: environment validation, info, status, version listing,
//! and install/update/uninstall.

use std::fs;
use std::path::Path;

use crate::harness::*;

// ---------------------------------------------------------------------------
// `oqtopus manager` — environment validation, info, status
// ---------------------------------------------------------------------------

#[test]
fn manager_info_reports_metadata_verbatim() {
    let context = TestContext::new();
    EnvFixture::manager(&context, &[]);

    let output = context.run(["manager", "info"], 0);

    snap!("manager_info__metadata", output);
}

#[test]
fn manager_environment_validation_errors() {
    // No `.metadata` at all.
    {
        let context = TestContext::new();
        let output = context.run(["manager", "info"], 1);
        snap!("manager_env__no_metadata", output);
    }

    // `.metadata` exists but belongs to a different template.
    {
        let context = TestContext::new();
        EnvFixture::backend(&context, &[]);
        let output = context.run(["manager", "info"], 1);
        snap!("manager_env__wrong_template", output);
    }
}

#[test]
fn manager_status_reports_stopped_without_pid_file() {
    let context = TestContext::new();
    EnvFixture::manager(&context, &[]);

    let output = context.run(["manager", "status"], 0);

    snap!("manager_status__stopped", output);
}

// ---------------------------------------------------------------------------
// `oqtopus manager versions`
// ---------------------------------------------------------------------------

#[test]
fn manager_versions_lists_remote_tags_outside_an_environment() {
    let context = TestContext::new();
    context
        .fake_tools
        .fixture("curl")
        .stdout(advertised_tags(&["v1.2.3", "v2.0.0", "v1.10.0", "nightly"]));

    let output = context.run(["manager", "versions"], 0);
    assert_eq!(context.fake_tools.call_count("curl"), 1);

    snap!("manager_versions__outside_environment", output);
}

#[test]
fn manager_versions_annotates_environment_context() {
    let tags = advertised_tags(&["v1.2.3", "v2.0.0", "v1.10.0", "nightly"]);

    // The current binding is the only difference between the two cases: it
    // moves the `* ` marker, and a branch binding sorts ahead of every tag.
    for (name, manager_version) in [
        ("manager_versions__current_release", "v1.10.0"),
        ("manager_versions__current_branch", "branch:feature/x"),
    ] {
        let context = TestContext::new();
        let env = EnvFixture::manager(&context, &[("manager_version", manager_version)]);
        for release in [
            "manager-v1.2.3",
            "manager-v1.10.0",
            "manager-v3.3.3",
            "manager-vfoo",
        ] {
            env.install_release(release);
        }
        context.fake_tools.fixture("curl").stdout(&tags);

        let output = context.run(["manager", "versions"], 0);
        assert_eq!(context.fake_tools.call_count("curl"), 1);

        snap!(name, output);
    }
}

#[test]
fn manager_versions_fail_when_the_advertisement_has_no_tags() {
    // The remote answers, but advertises no tag refs at all. The CLI folds
    // this into the same error as a failed fetch.
    let context = TestContext::new();
    context
        .fake_tools
        .fixture("curl")
        .stdout(advertised_refs(&[]));

    let output = context.run(["manager", "versions"], 1);

    snap!("manager_versions__no_tags_in_advertisement", output);
}

// ---------------------------------------------------------------------------
// `oqtopus manager install / update / uninstall`
// ---------------------------------------------------------------------------

/// A minimal oqtopus-manager source checkout, as codeload would deliver it.
fn build_manager_checkout_archive(root: &Path, top_level: &str) -> Vec<u8> {
    build_targz(root, top_level, |contents| {
        fs::write(
            contents.join("pyproject.toml"),
            "[project]\nname = \"oqtopus-manager\"\n",
        )
        .expect("write manager pyproject fixture");
    })
}

#[test]
fn manager_install_release_downloads_the_tag_archive() {
    let context = TestContext::new();
    let env = EnvFixture::manager(&context, &[]);
    // A pinned release install skips ref resolution: the only curl call is
    // the tag tarball itself.
    let checkout = build_manager_checkout_archive(context.root(), "oqtopus-manager-1.2.3");
    context.fake_tools.fixture_call("curl", 1).stdout(checkout);
    context.fake_tools.fixture("uv");

    let output = context.run(["manager", "install", "v1.2.3"], 0);
    assert_eq!(context.fake_tools.call_count("curl"), 1);

    let release = env.install_root.join("manager-v1.2.3");
    assert!(
        release.join("pyproject.toml").is_file(),
        "release archive should be extracted with its top-level directory stripped"
    );
    assert!(
        release.join(".venv").is_dir(),
        "uv sync should be pointed at the extracted release"
    );

    if context.invoke_with_bash {
        snap!(
            "bash_external_calls__manager_install_release",
            normalize(&context.fake_tools.log(), context.root())
        );
    }

    snap!(
        "manager_install__release_binding",
        render_observation(&context, &output, &[".metadata"])
    );
}

#[test]
fn manager_install_branch_writes_metadata_binding() {
    let context = TestContext::new();
    EnvFixture::manager(&context, &[]);
    // A branch install makes two curl calls: the ref advertisement resolving
    // the branch to a commit id, then the codeload tarball for that commit.
    context
        .fake_tools
        .fixture_call("curl", 1)
        .stdout(advertised_refs(&["heads/develop"]));
    let checkout = build_manager_checkout_archive(context.root(), "oqtopus-manager-checkout");
    context.fake_tools.fixture_call("curl", 2).stdout(checkout);
    context.fake_tools.fixture("uv");

    let output = context.run(["manager", "install", "branch:develop"], 0);
    assert_eq!(context.fake_tools.call_count("curl"), 2);

    if context.invoke_with_bash {
        snap!(
            "bash_external_calls__manager_install_branch",
            normalize(&context.fake_tools.log(), context.root())
        );
    }

    snap!(
        "manager_install__branch_binding",
        render_observation(&context, &output, &[".metadata"])
    );
}

#[test]
fn manager_update_resolves_and_installs_the_latest_release() {
    let context = TestContext::new();
    EnvFixture::manager(&context, &[("manager_version", "v1.2.3")]);
    // `update` takes no version argument: the first curl call resolves the
    // latest stable tag, the second downloads that tag's tarball.
    context
        .fake_tools
        .fixture_call("curl", 1)
        .stdout(advertised_tags(&["v1.2.3", "v1.10.0", "v1.9.9"]));
    let checkout = build_manager_checkout_archive(context.root(), "oqtopus-manager-1.10.0");
    context.fake_tools.fixture_call("curl", 2).stdout(checkout);
    context.fake_tools.fixture("uv");

    let output = context.run(["manager", "update"], 0);
    assert_eq!(context.fake_tools.call_count("curl"), 2);

    snap!(
        "manager_update__rebinds_to_latest",
        render_observation(&context, &output, &[".metadata"])
    );
}

#[test]
fn manager_uninstall_branch_clears_metadata_binding() {
    let context = TestContext::new();
    EnvFixture::manager(&context, &[("manager_version", "branch:main")]);
    fs::create_dir_all(context.work_dir().join("manager")).expect("create manager branch checkout");

    let output = context.run(["manager", "uninstall", "branch:main"], 0);

    snap!(
        "manager_uninstall__branch_binding",
        render_observation(&context, &output, &[".metadata"])
    );
}
