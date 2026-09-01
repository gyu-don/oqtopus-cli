# Characterization tests for `bin/oqtopus`

This suite pins down the *observable behavior* of the Bash CLI so that a
reimplementation (in particular a planned Rust port) can be validated against
it. Every test runs the CLI in an isolated sandbox and records, as an
[insta](https://insta.rs) snapshot:

- the exit code,
- the exact stdout and stderr text (including trailing newlines),
- where relevant, the resulting file tree and selected file contents,
- and, for the Bash baseline only, the argv of every external command the CLI
  invoked.

If any of that changes, a snapshot diff appears. A snapshot change is a
behavior-contract change and should be reviewed as such — never re-blessed
without reading the diff.

## Running

```shell
make test          # everything, against bin/oqtopus (or OQTOPUS_TEST_BIN)
make test-bash     # characterization suite, forced onto the Bash CLI
```

To run the suite against a candidate reimplementation:

```shell
OQTOPUS_TEST_BIN=/absolute/path/to/rust-oqtopus \
  cargo test --locked --test characterization
```

With `OQTOPUS_TEST_BIN` set, the `bash_external_calls__*` snapshots are
skipped (the recorded argv is a Bash implementation detail); everything else —
exit codes, output text, files — must match the recorded baseline exactly.

## Layout

| File | Covers |
| --- | --- |
| `main.rs` | Crate root: module list only. |
| `harness.rs` | `TestContext` (sandboxed invocation), output rendering and normalization, `EnvFixture` (a pre-built environment's `.metadata`), and builders for curl payloads (ref advertisements, tar.gz archives). |
| `help.rs` | Help texts, usage screens, completion scripts, `version`. |
| `errors.rs` | The argument-error contract and missing-dependency errors. |
| `init.rs` | `oqtopus init`: name validation, backend/manager templates, `--branch`. |
| `versions.rs` | Remote tag discovery, semver filtering and numeric sorting, environment annotations, latest-version resolution. |
| `metadata.rs` | `.metadata` parsing quirks and the legacy `env_name`/`env_root` key migration. |
| `backend.rs` | `oqtopus backend`: environment validation, info, branch install/uninstall, sse-runtime build. |
| `manager.rs` | `oqtopus manager`: environment validation, info, status, versions, install/update/uninstall. |
| `../support/fake_tool.rs` | Test-facing API for configuring the fake external tools. |
| `../../src/bin/oqtopus-test-fake.rs` | The fake executable that stands in for `curl`, `date`, `docker`, and `uv`. |
| `snapshots/` | The recorded behavior. File names mirror the behavior they record (the harness's `snap!` macro drops insta's module prefix, so moving a test between modules never renames its snapshot). |

## How a test runs

1. `TestContext::new()` creates a temporary sandbox with its own `HOME`, XDG
   directories, `TMPDIR`, and a `work/` directory the CLI runs in. The
   environment is cleared; locale, timezone, and CLI version are pinned; proxy
   variables point at a dead port so nothing can reach the network.
2. Symlinks named `curl`, `date`, `docker`, and `uv` — all pointing at the
   `oqtopus-test-fake` binary — are prepended to `PATH`. A fake call without a
   configured fixture fails with exit 125, so a test can never silently fall
   through to a real tool. (`tar`, `mktemp`, `awk`, etc. remain real.)
3. The test configures fixtures: `fixture("curl").stdout(...)` answers every
   curl call; `fixture_call("curl", 2).stdout(...)` answers only the second
   one, for commands that fetch a ref advertisement first and an archive
   second. The fake `curl` honors `-o`, and the fake `uv sync` creates the
   project's `.venv`, mirroring the side effects the CLI relies on.
4. The CLI runs; `run(args, expected_code)` asserts the exit code and returns
   the rendered output. `render_observation` appends the `work/` file tree and
   selected file contents when on-disk effects matter.
5. Sandbox paths, temp names, and per-machine values (UID/GID) are normalized
   to placeholders such as `<TEST_ROOT>` before snapshotting, so snapshots are
   machine-independent.

The curl payload builders deserve a note: since the CLI switched from the
GitHub REST API to git smart-HTTP ref advertisements, tag/branch fixtures are
built by `advertised_tags`/`advertised_refs` in `harness.rs`. They produce a
realistically framed pkt-line response (length prefixes, `# service` preamble,
NUL-separated capability list) because tolerating that framing is part of the
behavior being characterized. Commit ids are fabricated deterministically from
the ref name so they can appear in snapshots.

## Updating snapshots

Bless changes from the Bash baseline only:

```shell
INSTA_UPDATE=always make test-bash
```

Then review the diff with `git diff` (or `cargo insta review`) and commit the
snapshot together with the change that caused it. If a snapshot changes when
you did not intend to change behavior, that is a regression, not a stale
snapshot.

## What is deliberately not covered

- Service start/stop/restart beyond `manager status` (backgrounding, PID
  files, and `sleep`-based polling make them slow and racy to characterize).
- `cloud-local start db` orchestration (docker compose health probes,
  alembic migrations, seeding).
- Real network behavior: every HTTP interaction is characterized at the
  "argv passed to curl + response consumed" boundary.
