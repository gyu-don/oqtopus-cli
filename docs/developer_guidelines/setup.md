
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

External commands the CLI shells out to (`curl`, `date`, `docker`, `git`, and `uv`) are
replaced by fake executables on `PATH` during tests. Each fake tool call is matched
against a configured fixture; a call with no fixture configured fails immediately with
exit code 125, so a test can never succeed implicitly by falling through to a real
external command. The shared snapshots capture exit status, stdout, stderr, the
resulting file tree, and the contents of selected files; the Bash-specific argv passed
to the fake tools is recorded and snapshotted separately.

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
