//! Remote version discovery: tag filtering and numeric sorting, environment
//! annotations, and latest-version resolution.

use std::fs;

use crate::harness::*;

#[test]
fn backend_versions_filters_and_sorts_stable_semver_tags() {
    let context = TestContext::new();
    // Branch refs and the peeled `^{}` companion of an annotated tag must be
    // ignored; non-semver and pre-release tags must be filtered out.
    let refs = advertised_refs(&[
        "heads/feature/x",
        "tags/v1.2.3",
        "tags/v2.0.0-rc.1",
        "tags/v10.0.0",
        "tags/nightly",
        "tags/v1.10.0",
        "tags/v2.0.0",
        "tags/v2.0.0^{}",
        "tags/v0.9.9",
        "tags/release-v4.0.0",
    ]);
    context.fake_tools.fixture("curl").stdout(refs);

    let output = context.run(["backend", "versions", "engine"], 0);

    if context.invoke_with_bash {
        assert_eq!(context.fake_tools.call_count("curl"), 1);
        assert_eq!(context.fake_tools.call_count("date"), 0);
        snap!(
            "bash_external_calls__backend_versions",
            normalize(&context.fake_tools.log(), context.root())
        );
    }

    snap!(
        "backend_versions_filters_and_sorts_stable_semver_tags",
        render_observation(&context, &output, &[])
    );
}

#[test]
fn backend_versions_annotates_environment_context() {
    let tags = advertised_tags(&[
        "v1.2.3",
        "v2.0.0",
        "v1.10.0",
        "v0.9.9",
        "v2.0.0-rc.1",
        "nightly",
    ]);

    // The current binding is the only difference between the two cases: it moves
    // the `* ` marker, and a branch binding sorts ahead of every semver tag.
    for (name, engine_version) in [
        ("backend_versions__current_release", "v1.10.0"),
        ("backend_versions__current_branch", "branch:feature/x"),
    ] {
        let context = TestContext::new();
        let env = EnvFixture::backend(&context, &[("engine_version", engine_version)]);
        for release in [
            "engine-v1.2.3",
            "engine-v1.10.0",
            "engine-v3.3.3",
            "engine-vfoo",
            "tranqu-v1.0.0",
        ] {
            env.install_release(release);
        }
        context.fake_tools.fixture("curl").stdout(&tags);

        let output = context.run(["backend", "versions", "engine"], 0);
        assert_eq!(context.fake_tools.call_count("curl"), 1);

        snap!(name, output);
    }
}

#[test]
fn backend_versions_sorts_patch_releases_numerically() {
    // 100 tags with a shared major.minor so the only thing distinguishing
    // entries is `.patch`. This is deliberately "adversarial" for a naive
    // lexicographic sort: v1.0.99 < v1.0.9 as strings, but must sort *after*
    // it numerically. A full descending dump of all 100 versions is the only
    // way to confirm the sort is numeric end-to-end (e.g. that v1.0.10 lands
    // between v1.0.11 and v1.0.9, not next to v1.0.1).
    let tags: Vec<String> = (0..100).map(|patch| format!("v1.0.{patch}")).collect();
    let refs = advertised_tags(&tags.iter().map(String::as_str).collect::<Vec<_>>());

    let context = TestContext::new();
    context.fake_tools.fixture("curl").stdout(refs);

    let output = context.run(["backend", "versions", "gateway"], 0);
    assert_eq!(context.fake_tools.call_count("curl"), 1);

    snap!("backend_versions_sorts_patch_releases_numerically", output);
}

#[test]
fn version_resolves_latest_when_cli_version_is_empty() {
    // Same adversarial patch-release fixture as list_component_versions, but
    // exercised through `resolve_latest_version`, which must pick the numeric
    // maximum as "latest".
    let tags: Vec<String> = (0..100).map(|patch| format!("v1.0.{patch}")).collect();
    let refs = advertised_tags(&tags.iter().map(String::as_str).collect::<Vec<_>>());

    let context = TestContext::new();
    context.fake_tools.fixture("curl").stdout(refs);

    // OQTOPUS_CLI_VERSION is always set by the harness; override it with the
    // empty string so `cli_version()` falls through to `resolve_latest_version`.
    let output = context.run_with_env(["version"], 0, [("OQTOPUS_CLI_VERSION", "")]);
    assert_eq!(context.fake_tools.call_count("curl"), 1);

    snap!("version_resolves_latest_when_cli_version_is_empty", output);
}

#[test]
fn backend_versions_rejects_unusable_input() {
    let context = TestContext::new();
    context
        .fake_tools
        .fixture("curl")
        .stdout(advertised_tags(&["nightly"]));
    let output = context.run(["backend", "versions", "tranqu"], 1);
    assert_eq!(context.fake_tools.call_count("curl"), 1);
    snap!("backend_versions__no_stable_versions", output);

    // `is_in_list` rejects the component before `need_command curl` ever runs,
    // so no curl fixture is configured here and none must be called.
    let context = TestContext::new();
    let output = context.run(["backend", "versions", "bogus"], 1);
    assert_eq!(context.fake_tools.call_count("curl"), 0);
    snap!("backend_versions__unknown_component", output);
}

#[test]
fn backend_versions_ignores_unusable_metadata() {
    let context = TestContext::new();
    let work = context.work_dir();
    fs::create_dir_all(&work).expect("create work dir");
    // Wrong template: `try_load_backend_env` must reject this silently rather
    // than warn, since "versions" works fine outside any environment too.
    fs::write(
        work.join(".metadata"),
        format!(
            "template=cloud-local\nenvironment_root={}\n",
            work.display()
        ),
    )
    .expect("write .metadata");
    context
        .fake_tools
        .fixture("curl")
        .stdout(advertised_tags(&["v1.2.3", "v2.0.0"]));

    let output = context.run(["backend", "versions", "engine"], 0);

    snap!("backend_versions__unusable_metadata", output);
}
