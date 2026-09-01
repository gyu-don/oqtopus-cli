//! `.metadata` parsing quirks and the legacy key migration.

use crate::harness::*;

#[test]
fn legacy_metadata_keys_are_migrated_in_place() {
    // Environments created by older CLI releases carry `env_name`/`env_root`.
    // Any command that validates the environment (here: `backend info`) must
    // both accept them and rewrite the file to the new key names.
    let context = TestContext::new();
    EnvFixture::create_legacy(&context, "backend", "backend", &[]);

    let output = context.run(["backend", "info"], 0);

    snap!(
        "legacy_metadata_keys__migrated_by_backend_info",
        render_observation(&context, &output, &[".metadata"])
    );
}

#[test]
fn legacy_metadata_keys_are_read_but_not_migrated_without_validation() {
    // `backend versions` only *tries* to load the environment, and the
    // read-only path must not rewrite `.metadata` as a side effect.
    let context = TestContext::new();
    EnvFixture::create_legacy(
        &context,
        "backend",
        "backend",
        &[("engine_version", "v1.2.3")],
    );
    context
        .fake_tools
        .fixture("curl")
        .stdout(advertised_tags(&["v1.2.3", "v2.0.0"]));

    let output = context.run(["backend", "versions", "engine"], 0);

    snap!(
        "legacy_metadata_keys__read_only_by_backend_versions",
        render_observation(&context, &output, &[".metadata"])
    );
}

#[test]
fn metadata_get_uses_the_first_match_and_keeps_inner_separators() {
    let tags = advertised_tags(&["v1.2.3", "v2.0.0"]);

    // Two `engine_version` lines in `.metadata`: the first one must win.
    {
        let context = TestContext::new();
        EnvFixture::backend(
            &context,
            &[("engine_version", "v1.2.3"), ("engine_version", "v2.0.0")],
        );
        context.fake_tools.fixture("curl").stdout(&tags);

        let output = context.run(["backend", "versions", "engine"], 0);

        snap!("metadata_get__duplicate_key", output);
    }

    // A value that itself contains `=`: everything after the first `=` on the
    // line is the value.
    {
        let context = TestContext::new();
        EnvFixture::backend(&context, &[("engine_version", "branch:a=b")]);
        context.fake_tools.fixture("curl").stdout(&tags);

        let output = context.run(["backend", "versions", "engine"], 0);

        snap!("metadata_get__value_contains_separator", output);
    }
}
