# Rust CLI Migration Design

## Purpose

The OQTOPUS CLI is moving from Bash to Rust without a flag-day rewrite. Rust
becomes the user-facing entrypoint first, while commands that have not yet been
ported continue to run through the existing Bash CLI.

This document records rules that are specific to that staged migration. Normal
implementation choices remain with the person or agent implementing each
slice. Progress tracking belongs in issues, and command-specific compatibility
decisions belong in the pull request that makes them.

## Migration architecture

The Rust executable owns a small, explicit routing decision:

1. A migrated command is dispatched to its native Rust implementation.
2. Every other invocation is passed to the legacy Bash CLI.

Initially every invocation goes to Bash. The native surface then grows one
command or coherent subcommand area at a time.

Fallback must be selected before strict Rust-side parsing. Arguments belonging
to Bash must not be rejected, normalized, reordered, or reconstructed by Rust.
Unknown commands and unusual legacy argument forms therefore default to Bash.

On supported platforms, fallback replaces the Rust process with `exec`. This
preserves the raw arguments, environment, working directory, standard streams,
signals, and exit behavior as closely as possible. Fallback is an entrypoint
concern and must not be invoked from native command logic.

The route inventory must be visible and reviewable. Changing a route from Bash
to Rust is an explicit migration step, not an incidental result of adding an
implementation.

## Command migration workflow

For each command or coherent subcommand slice:

1. Inspect the Bash implementation, documentation, and relevant usage.
2. Add the characterization cases needed to understand that slice.
3. Run those cases against Bash and review the resulting snapshots before
   implementing the Rust version.
4. Decide for each observed behavior whether to preserve it, intentionally
   change it, omit it as a Bash-specific detail, or leave it undecided.
5. Implement the Rust slice and add sufficient tests for its argument handling,
   main behavior, errors, and externally observable results.
6. Run migrated-route tests with Bash fallback forbidden.
7. Switch the explicit route to Rust only after the behavior decisions and
   tests are complete.

An undecided behavior blocks the route switch. Intentional compatibility
changes must be documented with the implementing change.

The characterization suite grows through this workflow. Completing broad
characterization coverage before Rust implementation begins is not a goal.

## Snapshot policy

Snapshots are compatibility evidence, not an automatic declaration that every
observed Bash behavior is permanent.

Use snapshots when reviewing a complete observable artifact is useful, such as
human-readable command output or generated files. Prefer focused assertions or
unit tests when they express parsing, ordering, validation, or other individual
semantics more clearly.

Do not preserve calls to `curl`, `jq`, `tar`, or other Bash implementation
details merely because the legacy CLI makes them. External calls that remain
part of product behavior, such as `uv` or Docker invocations, may be tested when
their arguments or effects matter.

Snapshot expectations must be produced from the Bash implementation before the
corresponding Rust implementation is written. Snapshot changes are reviewed;
they are never accepted mechanically.

The initial Rust wrapper deliberately has zero characterization snapshots.

## Test requirement

Every migrated slice must include tests sufficient to validate its argument
handling, main behavior, error handling, and externally observable results.
Compatibility-sensitive output and artifacts should use snapshots; other
behavior should use whichever focused tests communicate the expectation most
clearly.

Tests for a migrated route must be able to forbid Bash fallback. This prevents
a hybrid-entrypoint test from passing without exercising the Rust
implementation.

This design does not prescribe a test count, module layout, mocking strategy,
or a fixed ratio of snapshot, integration, and unit tests.

## Completion during the hybrid period

Shell completion must describe both migrated and legacy commands throughout
the hybrid period. It may initially remain a Bash route. Any later change in
its authoritative command model must be explicit and tested as part of the
slice that changes it.

## Delivery sequence

1. Introduce the Rust executable with every invocation delegated to Bash and
   an empty characterization test target wired to `insta`.
2. Choose a command slice and establish its reviewed Bash snapshots.
3. Implement and test that slice in Rust with fallback forbidden.
4. Switch its route to Rust.
5. Repeat until the explicit legacy route inventory is empty.
6. Remove the fallback only through a separate, deliberate decision.

The historical characterization branch may be consulted if useful, but this
plan does not depend on reusing it.

## Fallback retirement

The Bash implementation can be removed only after:

- every supported route is explicitly native;
- migrated-route tests cannot silently use fallback;
- compatibility decisions and intentional deviations have been recorded;
- completion covers the intended command surface; and
- distribution and rollback no longer depend on the legacy script.

## Open migration decisions

- Where the Rust executable finds the Bash fallback in development and in
  packaged installations.
- Which non-Unix platforms must be supported during the hybrid period and what
  replaces `exec` there.
- The granularity and representation of the route inventory once the first
  native slice is introduced.
- When completion moves from Bash to a Rust-owned command model.
- How the hybrid executable and legacy script are packaged, installed, and
  rolled back.
