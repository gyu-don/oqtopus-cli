# Rust CLI Migration Design

## Status and scope

This is a living, repository-internal design memo for migrating the OQTOPUS
CLI from Bash to Rust. It records the intended architecture, invariants,
delivery sequence, and decisions that remain open. It is deliberately outside
the public MkDocs `docs/` tree.

Progress checklists belong in issues. Command-specific compatibility decisions
and intentional deviations belong in the pull request that implements them or
in a linked decision record. The mechanics of the existing test harness remain
documented in `tests/characterization/README.md`. After the migration settles,
durable guidance can be distilled into public or developer documentation.

This memo does not design or implement the Rust port in detail. In particular,
it does not choose a complete crate layout, CLI parser, HTTP client, JSON
schema, release service, or rollback mechanism.

## Current facts

- The production CLI is `bin/oqtopus`, implemented in Bash.
- A substantial Rust characterization harness already runs the CLI in an
  isolated environment and records exit status, stdout, stderr, selected
  filesystem effects, and some external-command invocations as snapshots.
- The suite is useful but is not assumed complete. Coverage gaps remain, and
  not every existing snapshot has been manually accepted as desirable product
  behavior.
- `OQTOPUS_TEST_BIN` already allows the characterization suite to exercise a
  candidate executable instead of the Bash implementation.
- The product legitimately invokes `uv` and Docker. The Bash implementation
  also relies on shell tools such as `curl`, `jq`, and `tar` for work that Rust
  libraries can usually perform directly.

The characterization suite is therefore a behavioral baseline and comparison
oracle during the port. It is not a declaration that every observed Bash
behavior is a permanent contract. Each behavior becomes durable only after it
has been reviewed in the context of the command being migrated.

## Goals and non-goals

### Goals

- Make Rust the official front-door executable early, without requiring a
  flag-day rewrite.
- Migrate one coherent command slice at a time while retaining a faithful Bash
  fallback for everything else.
- Preserve current human-readable behavior by default while allowing explicit,
  reviewed improvements.
- Keep the implementation and its tests understandable to a human reviewer.
- Support additional command kinds and templates without prematurely building
  a plugin framework.
- Create a clean path to structured output for selected commands.
- Distribute a usable binary without requiring users to install Cargo.

### Non-goals

- Reimplement every command before Rust becomes the entrypoint.
- Mechanically turn every snapshot into a Rust unit test.
- Complete all characterization coverage before migration begins.
- Preserve known errors, inconsistencies, or misleading behavior merely
  because the Bash implementation currently exhibits them.
- Design a generic command registry, plugin system, universal abstraction
  layer, or final release pipeline before real requirements justify it.

Human comprehensibility is a primary constraint. Direct code with explicit
routing and narrow interfaces is preferred over speculative frameworks and
excessive abstraction.

## Chosen migration architecture

### Rust as the front door

The installed `oqtopus` executable should become Rust early in the migration.
At startup it performs a small, explicit routing decision:

1. If the requested command has migrated, dispatch it to the native Rust
   implementation.
2. Otherwise, hand the invocation to the legacy Bash CLI.

The fallback decision must happen before strict Rust-side parsing of a legacy
command. Rust must not reject, normalize, reorder, or reconstruct arguments
that belong to Bash. The fallback receives the raw argument vector and current
environment.

Where the platform supports it, fallback should replace the Rust process using
`exec` semantics. This most closely preserves argv, environment, stdin,
stdout, stderr, signal handling, and exit status. On platforms where direct
replacement is unavailable, the closest subprocess behavior must be defined
and tested rather than silently treated as equivalent.

Routing remains an entry-layer responsibility. Bash fallback calls must not be
scattered through application or business logic. Initially, every route falls
back to Bash. The fallback surface then narrows command by command until it can
be removed.

### Explicit route inventory

The route inventory must be easy to inspect in code and review in a diff. Each
command or coherent subcommand area is visibly marked as native or legacy. A
route switches to Rust only when its migration slice meets the testing and
review gates below.

Unknown commands and unusual legacy argument forms also need an explicit
routing policy. In the initial hybrid, the safe compatibility default is to
send anything not positively identified as native to Bash.

### Completion during the hybrid period

Shell completion must continue to cover both migrated and unmigrated commands.
Runtime routing and the command model used to generate completion have related
but distinct responsibilities; they must not be allowed to drift silently.

The implementation should establish which source describes the full command
surface during each phase and test generated completion against that surface.
This may initially mean delegating completion to Bash, later replacing it with
Rust generation when Rust has an authoritative model. The exact transition is
an open design choice, but completion is a migration requirement rather than
cleanup after the port.

## Implementation boundaries

Prefer one clear module per command, subcommand, or coherent command area. A
reader should be able to trace routing, validation, application behavior, and
rendering without traversing a generic framework.

Keep command/application logic separate from these boundaries:

- HTTP and remote-reference access;
- filesystem and archive operations;
- template acquisition and expansion;
- external process invocation; and
- human-readable or structured output rendering.

Introduce narrow interfaces where variability or testability is real, such as
a template source or process executor. Do not introduce a plugin system or
generic command registry in anticipation of future commands. New command kinds
and templates should fit through clear modules and specific shared services;
repeated concrete needs can justify further abstraction later.

Use Rust libraries instead of invoking `curl`, `jq`, or `tar` wherever
practical. `uv` and Docker may remain external dependencies because invoking
them is part of operating the product. Their process calls must be centralized
and testable, including argv, working directory, environment, and stdin where
relevant.

Portable prebuilt release artifacts are the desired distribution model; users
must not need Cargo to install or run the CLI. The release pipeline itself is
not yet designed. When choosing an HTTP/TLS stack, prefer a configuration that
does not accidentally impose a system OpenSSL dependency when an appropriate
portable alternative exists.

## Output and compatibility policy

Current human-readable output, stdout/stderr separation, and exit behavior are
the default compatibility target. Byte-for-byte preservation is not promised
for behavior that has been deliberately reviewed and changed.

Change output only for an explicit reason, such as correcting an error,
removing an inconsistency, clarifying misleading behavior, or improving a
material usability problem. Record every such choice as an intentional
compatibility deviation in the implementing pull request or linked decision
record, with corresponding test changes.

Business logic must not print directly. It should return typed results and
typed errors. A compatibility text renderer turns those values into the
current human-facing format and selects stdout or stderr. Selected commands
can later add `--json` through a separate structured renderer over the same
typed data; JSON output must never be produced by parsing human-readable text.

The exact JSON success and error schemas, exit behavior for structured errors,
and the set of commands that support `--json` remain open decisions.

## Testing and behavior review

### Confidence layers

Maintain distinct layers rather than asking one suite to prove everything:

1. **Bash baseline:** run the full characterization suite against the legacy
   Bash CLI.
2. **Hybrid entrypoint:** run the full suite against the Rust front door with
   native routing plus Bash fallback.
3. **Rust-only migrated routes:** test migrated commands with fallback
   forbidden, so an accidental delegation cannot produce a false pass.
4. **Router and fallback tests:** verify route selection, raw argument
   preservation, environment and stdio behavior, exit status, signals where
   practical, and failure behavior when fallback is unavailable.
5. **Focused Rust tests:** directly test parsing, validation, ordering,
   metadata, template handling, rendering, and other application logic.

Where it provides useful compatibility evidence, compare external-command
invocations as well as user-visible output. This is particularly important for
centralized `uv` and Docker execution.

### Incremental characterization

Do not mechanically translate the snapshot suite into unit tests or attempt a
massive up-front coverage pass. When a command is selected for migration,
inspect its Bash implementation, documentation, usage, existing tests, and
real scenarios. Add missing black-box cases needed to understand that slice,
then add focused Rust tests for its internal semantics.

Review behavior in human-sized slices: one subcommand or coherent scenario
group, roughly five to ten cases at a time. Classify each observation as:

- **preserve** — accepted compatibility behavior;
- **intentionally change** — replace it and document why;
- **remove / not relevant** — Bash detail that should not survive; or
- **undecided** — do not switch the route until resolved.

Reduce large snapshots to representative black-box observations when every
line is not itself the contract. Detailed ordering, parsing, and boundary
semantics belong in focused tests. Exact help output, generated files, and
other genuinely user-visible artifacts may remain snapshots when reviewing a
whole artifact is meaningful.

Never blindly re-bless snapshots. A snapshot diff represents a behavior
change and requires review, whether the cause is a Bash change, a new Rust
implementation, or a test cleanup.

The name `characterization` remains accurate during migration. After behavior
has been reviewed and Rust is authoritative, durable accepted cases may be
described as contract, regression, or acceptance tests. No rename is required
before that distinction is useful and true.

## Delivery sequence

### Phase 1: Record the design

Maintain this memo and make unresolved decisions visible. Track implementation
progress in issues rather than turning this document into a volatile task list.

### Phase 2: Improve test readability

Make the current characterization tests easier to review without changing
behavior. Begin with a small area such as version listing and resolution.
Split large modules by subcommand or coherent scenario when that aids reading,
and keep test names aligned with snapshot names. Work in the review-sized
slices described above.

### Phase 3: Introduce the hybrid entrypoint

Add the Rust executable, explicit router, and Bash fallback with all commands
still routed to Bash. Add router/fallback tests, a way for Rust-only tests to
forbid fallback, and CI coverage for both the Bash baseline and hybrid
entrypoint.

### Phase 4: Establish supporting delivery paths

Keep completion working and establish completion generation as needed for the
hybrid. Add enough prebuilt-binary and release scaffolding to test realistic
installation and rollback assumptions, without prematurely fixing the final
release pipeline.

### Phase 5: Port vertical slices

For each command or coherent subcommand slice:

1. Review its implementation, documented usage, and existing snapshots.
2. Add only the missing characterization cases needed for that slice.
3. Classify observed behaviors and document intentional deviations.
4. Implement the application logic, boundary adapters, and renderers in Rust.
5. Add focused Rust tests and run Rust-only tests with fallback forbidden.
6. Run the Bash baseline and hybrid suites, then switch that explicit route to
   Rust.

### Phase 6: Retire the fallback

Remove the Bash fallback and legacy implementation only after the completion
criteria below are satisfied. Removal is an explicit project decision, not an
automatic consequence of the last route being implemented.

## Invariants during migration

- Rust is the single user-facing entrypoint once the hybrid phase begins.
- A fallback-bound invocation reaches Bash before strict Rust parsing can alter
  or reject its arguments.
- Native business logic never invokes the Bash fallback.
- The route inventory is explicit; unknown or undecided routes default to Bash
  until deliberately migrated.
- Fallback is forbidden in Rust-only tests for migrated routes.
- Human output is rendered from typed results rather than printed throughout
  business logic.
- Structured output is rendered from typed data, not scraped from text.
- External process invocation is centralized and tested.
- Completion represents the intended full command surface throughout the
  hybrid period.
- Snapshot changes are reviewed, never mechanically accepted.
- User installation does not depend on Cargo.

## Completion criteria

The Bash implementation and fallback can be removed only when all of the
following are true:

- the route inventory is complete and every supported route is native;
- non-Bash-specific characterization expectations pass against Rust;
- known coverage gaps have been reviewed for every migrated command area;
- intentional compatibility deviations are documented;
- focused tests cover important Rust parsing, validation, application, and
  boundary behavior;
- completion is generated and validated from an authoritative command model;
- fallback has not been needed during an agreed validation period;
- prebuilt distribution, release, rollback, and legacy-removal concerns have
  been resolved; and
- maintainers explicitly accept Rust as the authoritative behavior.

## Open questions

- Where and how should the Rust executable locate the legacy Bash fallback in
  development, packaged artifacts, and rollback scenarios?
- Which non-`exec` platforms must the hybrid support, and what subprocess
  semantics are acceptable there?
- What is the authoritative completion model in each migration phase, and when
  should generation move from Bash to Rust?
- Which Rust argument parser and crate/module layout best preserve the explicit
  pre-parse fallback boundary?
- Which HTTP, TLS, archive, and template libraries best meet portability and
  readability requirements?
- What are the JSON success and error schemas, which commands support them,
  and how do structured errors interact with exit codes and stderr?
- What prebuilt artifact formats, platforms, signing or verification measures,
  installation path, and release automation are required?
- How long is the validation window in which fallback must remain unused
  before removal?
- What rollback guarantees are required when the Rust executable becomes the
  official entrypoint and when Bash is finally removed?
