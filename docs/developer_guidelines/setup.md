
# Development Environment Setup

This guide explains how to set up the development environment for contributing to OQTOPUS CLI.
The project provides a **Makefile** to simplify common development tasks.

## Prerequisites

Install the following tools before starting development.

| Tool                                        | Version | Description                        |
| ------------------------------------------- | ------- | ---------------------------------- |
| [Python](https://www.python.org/downloads/) | >=3.13  | Python programming language        |
| [uv](https://docs.astral.sh/uv/)            | >=0.10  | Python package and project manager |
| [Rust](https://rustup.rs/)                   | stable  | Characterization test toolchain    |

Clone the repository:

```shell
git clone https://github.com/oqtopus-team/oqtopus-cli.git
cd oqtopus-cli
```

## Project Structure

The repository is organized as follows:

```text
oqtopus-cli/
├─ bin/           # Production Bash CLI
├─ docs/          # Documentation sources (MkDocs)
├─ tests/         # Rust integration and snapshot tests
├─ .vscode/       # VSCode settings
├─ .github/       # GitHub workflows and repository settings
├─ Cargo.toml     # Rust test harness configuration
├─ Cargo.lock     # Locked Rust dependencies
├─ pyproject.toml # Project configuration and dependencies
├─ Makefile       # Development commands
├─ mkdocs.yml     # MkDocs configuration
├─ uv.lock        # Locked dependency versions
└─ README.md      # Project overview
```

## Installing Dependencies

Install the project dependencies and set up the local development environment:

```shell
make install
```

This command performs the following:

- Installs all dependencies via `uv`.
- Configures the Git commit message template.

Rust dependencies are downloaded automatically when the test suite is first run.

## Characterization Tests

The Rust test harness records the observable behavior of an OQTOPUS CLI executable.
It replaces external commands with test doubles, so tests must not use a real Docker
daemon or make network requests.

Run all Rust tests:

```shell
make test
```

Run only the characterization suite against the production Bash implementation:

```shell
make test-bash
```

The characterization harness uses `bin/oqtopus` when `OQTOPUS_TEST_BIN` is unset.
Set it to another executable to exercise a Rust implementation or another local build:

```shell
OQTOPUS_TEST_BIN="/absolute/path/to/rust-oqtopus" \
  cargo test --locked --test characterization
```

Use an absolute path so tests continue to find the executable after changing into a
temporary fixture directory.

Side-effect cases expose implementation-neutral test seams for a future native Rust
HTTP client and clock. A candidate should prefer the loopback endpoints when present,
fall back to the corresponding fixture file when sockets are unavailable, and use the
fixed timestamp:

| Environment variable | Test contract |
| --- | --- |
| `OQTOPUS_GITHUB_API_BASE_URL` | Base URL for GitHub API requests |
| `OQTOPUS_GITHUB_BASE_URL` | Base URL for GitHub archive downloads |
| `OQTOPUS_TEST_GITHUB_TAGS_FIXTURE` | Tags JSON fallback fixture path |
| `OQTOPUS_TEST_TEMPLATE_ARCHIVE_FIXTURE` | Template archive fallback fixture path |
| `OQTOPUS_TEST_NOW` | Fixed RFC 3339 current time |

These variables are supplied by the harness. They are compatibility seams for black-box
tests, not user configuration. The shared snapshots contain only exit status, output,
and filesystem state; Bash-specific `curl` and `date` argv are recorded separately.

### Updating Snapshots

Snapshot changes are part of the behavior contract. Review the CLI output first, then
regenerate the relevant snapshots deliberately. For the Bash baseline, run:

```shell
INSTA_UPDATE=always make test-bash
```

Snapshots are always blessed from the production Bash implementation. To compare a
Rust candidate, keep the executable selection explicit and do not set
`INSTA_UPDATE`:

```shell
OQTOPUS_TEST_BIN="/absolute/path/to/rust-oqtopus" \
  cargo test --locked --test characterization
```

If an intentional product change alters the contract, update the Bash baseline first,
inspect the snapshot diff, and then run both implementations against it. Rerun the Bash
suite without `INSTA_UPDATE` before committing. If `cargo-insta` is installed,
`cargo insta review` can be used for an interactive review instead.

## Documentation

### Lint Documentation

Run documentation linting:

```shell
make docs-lint
```

### Build Documentation

Build the documentation:

```shell
make docs-build
```

### Start the Documentation Server

This project uses [MkDocs](https://www.mkdocs.org/) to generate the HTML documentation and
Start the documentation server with:

```shell
make docs-serve
```

Open the documentation in your browser at [http://localhost:8000](http://localhost:8000).
