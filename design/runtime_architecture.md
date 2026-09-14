# Rust CLI runtime architecture

## Scope and completion criteria

This design covers the review cleanup for PR #35: module responsibilities,
UTF-8 text handling, and recoverable process startup. It describes the intended
maintained code, independently of the temporary Bash routing mechanism.

The cleanup is complete when:

- command dispatch lives in `cli`, with only process-wide setup and exit in
  `main`;
- each domain owns its command orchestration and service/component definitions,
  while shared modules implement reusable mechanisms;
- metadata has one UTF-8 representation and one set of editing functions;
- simultaneous starts cannot launch duplicate processes, startup can recover
  after caller termination, and failed startup does not leave an untracked
  running service; and
- focused recovery tests and the existing compatibility suite pass.

Completion generation, Bash retirement, release packaging, and installer work
remain separate migration tasks. This cleanup does not enable those routes or
claim that the migration is ready to merge.

## Responsibilities and dependencies

`main` owns process-wide signal setup, argument acquisition, and process exit.
`cli` owns routing, command dispatch, stream selection, final rendering, and
mapping command outcomes to exit codes. The temporary legacy route is selected
here before any strict native parsing.

`backend`, `cloud_local`, and `manager` own their command argument handling,
environment requirements, component definitions, service command construction,
and sequencing of start/stop/restart or installation steps. These domains may
use submodules to keep related operations readable. A command is found under
its domain rather than in a catch-all module for every lifecycle operation.

Shared modules have concrete responsibilities:

- `args`: the argument checks every command family repeats, such as recognizing
  a usage request and rejecting a target outside a service inventory;
- `service`: process inspection, startup exclusion, PID publication, child
  execution, environment-file loading, and stopping a process;
- `operations`: reusable release/branch installation, synchronization, binding
  updates, removal, and runtime build mechanics;
- `environment` and `metadata`: environment validation and metadata editing;
- `remote` and `archive`: network/ref discovery and archive extraction;
- `text` and `progress`: result rendering and incremental notifications.

Shared command outcome types and small argument helpers may live in a common
module, but that module must not own domain orchestration. Final result data
remains separate from its rendering. Incremental notifications stay explicit,
with the existing stream and flush behavior. Shared process/install mechanisms
may emit progress while working; this is not a reason to introduce a generic
output framework.

Service membership has one authoritative inventory per domain. Status order,
startup order, and shutdown order are separate policies, named explicitly
where they differ. Sharing an inventory must not silently reorder commands or
output consumed by the Manager. Component-specific policy stays with the
domain; reusable execution steps belong in shared helpers.

The CLI's compiled version and remote component release discovery are different
responsibilities. Their similar names alone do not justify combining modules.

## Text and compatibility

Metadata and other text files use `String`/`&str`. Invalid UTF-8 is rejected at
the read boundary rather than interpreted through a lossy view or preserved
through a parallel byte-oriented editing implementation. Network archives and
other binary payloads continue to use byte buffers.

UTF-8 does not imply normalization: successful `info` output retains source
text, including unknown fields, ordering, and CRLF. Updating a metadata key
preserves unrelated text. Metadata replacement remains atomic so readers do
not observe a partially written file. Unsupported non-UTF-8 metadata behavior
is an intentional compatibility change, not a new snapshot baseline to accept
automatically.

The Manager boundary remains unchanged for supported input: arguments, result
text, ordering, exit codes, standard streams, incremental progress, and runtime
paths. Refactoring does not relax these contracts. Internal comments should
explain current requirements and invariants; historical Bash explanations belong
in migration decisions when they are needed to justify compatibility.

## Process startup and termination

Startup exclusion uses a nonblocking OS file lock on the persistent
`pids/.<service>.start.guard` file. The file must never be removed during normal
cleanup: removing it could allow two callers to lock different inodes. Because
it outlives every start, an existing guard is opened read-only, which needs no
write access to lock: a guard created by one user must not permanently lock out
another. The OS releases the lock when its last owning descriptor closes,
including after SIGKILL. The old `.<service>.start.lock` directory and owner PID remain for
diagnostics and legacy lock recovery; their cleanup is not the source of mutual
exclusion between Rust callers.

The lock covers stale PID cleanup, command preparation, spawn, PID publication,
and the immediate-exit check. A foreground service releases the startup lock
before waiting for completion. The guard descriptor is close-on-exec so a
running service does not retain the lock.

The child writes its PID before executing the service, using only
async-signal-safe operations between fork and exec. Until exec it inherits the
startup guard. This closes the interval where killing a parent after spawn but
before parent-side PID writing would leave an untracked service. If the caller
dies after publication, a retry observes that service and does not start a
second instance. A PID left by a dead service is removed on retry.

Recovery tests exercise real subprocesses and bounded readiness waits, including
concurrent starts, forced termination, stale legacy locks, and immediate startup
failure. Normal `Drop` cleanup remains useful, but cannot be relied upon for
termination recovery. This guarantees process-level recovery; power-loss
durability and a distributed/network-filesystem lock protocol are outside the
local CLI's scope.

## Deferred work

The review explicitly defers the cross-cutting error-handling redesign. Its
follow-up should retain underlying IO/network/archive errors, add operation and
path context, and centralize user-facing formatting without breaking the
Manager's exit/stream contract. Existing `String`/unit error APIs are retained
in this cleanup except for errors necessary to express UTF-8 rejection and
startup failures.

Adopting `clap`, structured JSON output, and a shared output trait remain
separate changes. If the temporary migration document is deleted after the
port, retain this runtime design and the permanent compatibility decisions in
developer documentation.
