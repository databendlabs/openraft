# OpenRaft Jepsen Tests

This directory contains Jepsen black-box tests for OpenRaft and the dedicated
RocksDB-backed application they exercise. The tests target the application
through its external APIs rather than OpenRaft's internal Rust APIs, so they
can validate externally observable behavior through real client requests,
process lifecycle, and network behavior.

These tests cover a different layer from OpenRaft's deterministic simulation tests. Simulation tests run inside a controlled Rust environment. Jepsen drives a running KV service through its external API and records a client history for later checking. Jepsen runs are not deterministic, and a Jepsen failure is not expected to replay directly in the simulation harness.

## Organization

The Clojure/Leiningen harness and its Rust test application are self-contained
inside the OpenRaft repository. The Rust crate is excluded from the root Cargo
workspace, so normal Rust builds and tests do not depend on Jepsen. The Jepsen
Docker environment builds the application explicitly from its own manifest.

The Docker test environment separates the Jepsen control plane from the OpenRaft data plane:

```text
                     control container
                     Jepsen + Leiningen
                              |
              SSH: setup / start / stop / logs
                              |
          +---------+---------+---------+---------+
          |         |         |         |         |
          v         v         v         v         v
         n1        n2        n3        n4        n5
      db node   db node   db node   db node   db node
          |         |         |         |         |
          +------ app_http API and Raft RPC mesh --+
```

The control container is not an OpenRaft member. It runs the Jepsen process,
controls the db nodes over SSH, and sends client operations to the KV service
API. Every db node exposes the same `app_http` endpoints, and the leader-aware
client may contact any of them. The default OpenRaft cluster contains all five
Docker nodes, which communicate over the Raft RPC port.

The intended layout is:

```text
jepsen/
  Makefile
  project.clj
  README.md
  openraft-test-app/
    Cargo.toml
    src/
  docker/
    docker-compose.yml
    control.Dockerfile
    control.Dockerfile.dockerignore
    node.Dockerfile
    node.Dockerfile.dockerignore
    init-ssh-key.sh
  src/jepsen/openraft/
    checker.clj
    cli.clj
    client.clj
    db.clj
    cluster.clj
    interruption.clj
    nemesis.clj
    nemesis/
      membership.clj
      partition.clj
      process.clj
    quorum.clj
    workload.clj
```

The `openraft-test-app` crate is derived from the RocksDB example but belongs to
the Jepsen harness. It uses string node IDs so each Docker hostname can also be
the corresponding OpenRaft node ID. Test-specific application changes can be
made there without increasing the complexity of the general-purpose example.

The `jepsen.openraft` namespace contains the OpenRaft-specific Jepsen code:

- `checker.clj`: run metadata, unhandled exception, and node log checkers.
- `cli.clj`: command-line entry point.
- `client.clj`: HTTP client for the OpenRaft KV example APIs.
- `db.clj`: Jepsen DB lifecycle for starting and stopping OpenRaft nodes.
- `cluster.clj`: cluster bootstrap helpers.
- `interruption.clj`: shared thread-interruption classification.
- `nemesis.clj`: fault scheduling, composition, and final recovery.
- `nemesis/membership.clj`: membership growth, shrink, and final restoration.
- `nemesis/partition.clj`: leader-aware network partition faults and recovery.
- `nemesis/process.clj`: quorum-safe process kill/restart and pause/resume faults.
- `quorum.clj`: stable and joint-consensus quorum calculations.
- `workload.clj`: generators and checkers for client operations.

## Running

### Prerequisites

- Docker with Compose support

### Run The Harness

From the repository root:

```bash
# Format and lint the dedicated Rust test application.
$ make -C jepsen app-lint

# Build images, start containers, then run unit and default chaos tests.
$ make -C jepsen jepsen

# Generate the local Docker SSH key and build the Jepsen images.
$ make -C jepsen build

# Start or recreate the Jepsen containers.
$ make -C jepsen up

# Run the default chaos test against the running containers.
$ make -C jepsen test

# Run only the network partition test.
$ make -C jepsen test NEMESIS=partition

# Run only the process crash/restart test.
$ make -C jepsen test NEMESIS=process

# Run only the process pause/resume test.
$ make -C jepsen test NEMESIS=pause

# Run only the membership change test.
$ make -C jepsen test NEMESIS=membership

# Compose selected fault classes with overlapping schedules.
$ make -C jepsen test NEMESIS=partition,process,pause

# Reuse a recorded seed for Jepsen random choices.
$ make -C jepsen test NEMESIS=partition SEED=123456

# Override the committed-log threshold that triggers snapshots.
$ make -C jepsen test SNAPSHOT_THRESHOLD=250

# Stop and remove the Jepsen containers.
$ make -C jepsen down
```

The harness uses each Jepsen node's container hostname, such as `n1`, directly
as its OpenRaft node ID.

This starts the five-node Docker environment, then runs the Jepsen control
process from the control container. Every test checks a concurrent mix of
linearizable reads, writes, and compare-and-set operations across independent
registers with Knossos. The default `chaos` profile independently
schedules partition, process, pause, and membership faults, so their active
intervals can overlap. `NEMESIS` accepts a comma-separated subset when a
narrower combination is needed.

The partition nemesis alternates between partitions where the current leader is
in the majority and in the minority. A focused partition run requires both
modes to occur.

The process nemesis reads the effective voter configs from OpenRaft metrics and
randomly stops a non-empty voter subset whose survivors still form a quorum. It
supports both stable and joint membership and covers leader and follower-only
crashes.

The pause nemesis uses the same quorum-safe target selection, but suspends the
selected processes without terminating them. Their in-memory state and open TCP
connections remain in place, so peers observe an unresponsive process rather
than a closed connection. It covers pauses that include and exclude the current
leader. Resume operations target every test node as an idempotent cleanup and
record both the preceding disruption and the complete resumed node set in the
Jepsen history.

The membership nemesis starts with a shrink and grow for deterministic coverage,
then randomly mixes additional membership changes. Removed nodes are stopped
and wiped only after the new voter set is committed. The final recovery restores
all five nodes as voters and waits for every node to agree on a leader.

Every Jepsen node builds a snapshot after 100 newly committed logs by default.
The regular write workload therefore exercises snapshot construction during
short fault tests without a snapshot-specific Nemesis. Set
`SNAPSHOT_THRESHOLD` to override the threshold for a run.

After the fault schedule ends, Jepsen heals partitions, restarts killed
processes, resumes paused processes, restores membership, and then performs one
shared readiness check. Client operations continue while faults are active and
during recovery. Final recovery performs one write and one read per register;
every operation must succeed.

### Outcome Semantics and Evidence

[Jepsen error-handling semantics](error-handling-semantics.md) defines the
agreed target contract. It is authoritative where the implementation notes
below differ.

An unsuccessful operation is often a correct and necessary Jepsen observation,
not a failure of the test itself. This document uses *outcome* for the result of
a Client or Nemesis operation, *SUT failure* for an OpenRaft property violation
established by a checker, and *Harness failure* for a defect in the test code,
configuration, or execution environment. An `:error` field in an EDN record is
diagnostic data; its presence alone does not decide which category applies. The
classification depends on whether the condition is a modeled external result,
not on whether it occurred before, during, or after a Client or Nemesis call.
There is no separate generic "Client error" category.

#### Workload Outcomes

Every Client invocation and completion belongs in `history.edn`:

- `:ok` means the operation completed successfully.
- `:fail` means the operation did not take effect.
- `:info` means a mutating operation may or may not have taken effect, so its
  result is indeterminate.

Connection failures, request timeouts, and version mismatches can therefore be
valid workload outcomes. Even when a Nemesis caused them, they remain workload
outcomes because they describe what the Client observed from the SUT. Workload
checkers interpret these outcomes together rather than treating every
unsuccessful request as a broken test. The Client converts only recognized
transport, HTTP, and OpenRaft API conditions into workload outcomes. A recognized
protocol violation, such as invalid JSON, is recorded as an unexpected outcome
for the workload checker. An exception that the Client does not recognize is a
Harness failure, not a catch-all `:client-error` outcome.

The current workload still has a catch-all `:client-error` fallback for unknown
exceptions. Its unexpected-error checker prevents such a run from passing, but
the fallback must be removed to enforce the boundary described above.

#### Nemesis Outcomes

Nemesis invocations and completions also belong in `history.edn`. Their outcomes
describe whether a requested fault was installed, skipped, failed, or became
indeterminate. For example, `:no-supported-leader` and
`:no-reachable-pause-target` mean that no fault was installed and must not count
toward fault coverage. A partial or uncertain fault leaves the affected cluster
state unknown until a later cleanup or recovery operation proves otherwise.

Nemesis operations use informational history events by convention, so their
`:type` alone does not establish success. The OpenRaft Nemesis checkers inspect
their structured values, errors, observed modes, and final recovery state. An
expected operational problem must be converted into a structured Nemesis
outcome. An unexpected implementation exception, such as a null dereference, is
a Harness failure even when it occurs while a Nemesis operation is running.

The partition, process, and pause wrappers treat exceptions from delegated fault
injection as Harness failures. Only explicitly modeled conditions, such as a
missing supported leader, become Nemesis outcomes. A new expected failure mode
requires a narrow classifier; delegate exceptions must not be caught by a
generic fallback.

#### Checker Verdicts

Checkers turn the collected evidence into separate verdicts. Property checkers
decide whether OpenRaft satisfies a property: for example, the linearizability
checker may prove that no legal sequential execution explains the Client
history, or the crash checker may find the stable panic marker in an OpenRaft
node log. Coverage checkers instead decide whether the run produced enough
evidence that required scenarios were successfully exercised, such as all
required Nemesis modes and successful main-phase operations. Nemesis checkers
also verify a separate lifecycle postcondition: after fault cleanup, the cluster
must return to the required state within the recovery deadline.

A coverage failure rejects the run but does not by itself identify an OpenRaft
failure or its root cause. For example, `:missing-modes` is written to
`results.edn`; the mode may be absent from `history.edn`, skipped by an outcome
such as `:no-supported-leader`, or blocked by a Harness failure recorded in
`jepsen.log`. Start with the nested checker result, inspect the corresponding
operations in `history.edn`, and then use `jepsen.log` for Harness diagnostics.

The top-level `:valid?` combines these independent verdicts and answers only
whether the entire run can be accepted. It does not by itself identify an
OpenRaft property violation or the state of the Harness.

#### Harness Failures

Harness failures include Client or Nemesis implementation bugs, invalid test
configuration, missing runtime dependencies, and unexpected failures in setup,
teardown, artifact storage, or checker execution. They make the run unacceptable
and must not be reclassified as ordinary workload or Nemesis outcomes. They do
not automatically erase a conclusion already established by a nested checker.
`jepsen.log` is the primary diagnostic record for these failures. Jepsen may also
attach a worker exception to the operation being executed in `history.edn`; that
association is diagnostic evidence, not a successful or expected outcome.

Best-effort teardown is different from formal fault cleanup. When cleanup and
recovery completions exist in `history.edn`, they determine the checker verdict;
teardown must not manufacture a successful outcome. An expected teardown
cleanup failure should be logged as a warning and must not prevent analysis of
the recorded history. An unexpected teardown implementation failure remains a
Harness failure.

The current partition and pause teardown implementations do not yet enforce
this boundary: they log every non-interruption exception as a cleanup warning.
They must be narrowed so unexpected implementation exceptions remain Harness
failures.

The required Harness policy is controlled fail-fast. The first unhandled
exception should be retained as the run-level cause, stop new Client and Nemesis
operations, run best-effort fault cleanup, preserve the available artifacts,
and exit nonzero. The run cannot pass, although individual property and coverage
checkers may still produce independent verdicts. A Harness failure does not by
itself prove an SUT failure.

Interruption is a cancellation mechanism, not an operation outcome or a
standalone failure category. Client and Nemesis code must restore the thread's
interrupt flag and rethrow instead of converting interruption into `:fail`,
`:info`, or a cleanup warning. Normal Jepsen completion does not use
interruption: after the generator is exhausted, the interpreter drains
outstanding operations and stops its workers. If the interpreter exits
abnormally, worker interruption is a secondary cleanup signal rather than the
original failure. Client and Nemesis code must therefore propagate interruption
without trying to infer its cause. Controlled fail-fast instead needs a
run-level channel for the first unhandled exception, not cancellation-origin or
Harness fields in `history.edn`.

The current implementation does not yet have this run-level failure channel or
stop the schedule immediately. Jepsen catches a worker exception, records an
`:info` completion with `:exception`, and continues running. The strict
unhandled-exception checker reports `:unknown`, so such a run cannot pass;
another nested checker may still make the composed top-level result `false`.
Controlled fail-fast remains necessary to report the Harness failure at the
point where it occurs.

#### Stored Evidence

A run normally stores its artifacts under
`jepsen/store/<test-name>/<timestamp>/`:

- `history.edn` is the structured Client and Nemesis history used by the
  workload, Nemesis, statistics, and unhandled-exception checkers. `history.txt`
  is a human-readable rendering of the same history.
- `jepsen.log` records Harness activity and diagnostics from setup, workers,
  fault injection, teardown, and analysis. Checkers do not use it as an
  observation source.
- `<node>/openraft.log` is the collected runtime log of the OpenRaft test
  application on that node. The crash checker scans it for the test
  application's stable panic marker.
- `results.edn` contains the composed checker results and top-level verdict. It
  is a summary, not the raw execution record.
- `test.jepsen` contains the original test map, including run options and the
  random seed. It remains useful when a Harness failure prevents `results.edn`
  from being written.

Use `history.edn` to reconstruct requests and the fault schedule, `jepsen.log`
to diagnose the Harness, and each node's `openraft.log` to investigate the
OpenRaft process. Start with `results.edn` when the run reached analysis and a
checker verdict is available.

### Interpreting Results

Each run writes its checker output to
`jepsen/store/<test-name>/<timestamp>/results.edn`. The top-level `:valid?` has
three possible values:

- `true` exits with status 0. Every checker accepted the run.
- `false` exits with status 1. At least one checker established a failing
  condition. Inspect `:workload`, `:nemesis`, `:crash`, and `:stats` to identify
  it. Only `[:workload :linearizable :valid?]` being `false` means one or more
  register histories were not linearizable.
- `:unknown` exits with status 2. The harness could not establish a conclusive
  result, so the run must not be treated as passing. Unhandled worker or Nemesis
  exceptions appear at `[:exceptions :exceptions]`. Missing node logs appear at
  `[:crash :missing-nodes]`. Individual workload and Nemesis checkers may also
  report `:unknown`; locate the nested checker with that validity for its
  diagnostic fields.

The test name begins with `openraft linearizable registers`, which also names
its directory under `jepsen/store`. The independent linearizability checker
lists failing keys at `[:workload :linearizable :failures]` and stores each
key's result at `[:workload :linearizable :results <key>]`. Per-key histories
and checker output are written under
`jepsen/store/<test-name>/<timestamp>/independent/<key>/`.

Every run also records its random seed in `results.edn`, in a checker result of
the following form:

```clojure
:seed {:valid? true, :seed 123456}
```

The original test map in `test.jepsen` is the fallback when `results.edn` is not
available. Supplying the seed again repeats choices made through
`jepsen.random`, including the random node selection used by the partition,
process, pause, and membership nemeses. It does not make the whole run
deterministic: client operation mixing and timing use separate generator
randomness, and thread, network, and election timing can still differ. The
recorded history and Nemesis operations are therefore the authoritative account
of the fault schedule; the seed is only an aid for rerunning similar conditions.

## Implemented coverage

- [x] Add the Leiningen project definition and CLI skeleton.
- [x] Add Docker-based Jepsen control and node containers.
- [x] Add a multi-stage node image for the RocksDB KV test application.
- [x] Add an HTTP client for the OpenRaft KV APIs.
- [x] Add Jepsen process lifecycle management for OpenRaft nodes.
- [x] Bootstrap a five-node OpenRaft cluster.
- [x] Record phase-aware ok, fail, and info counts for read, write, and CAS.
- [x] Add a network partition nemesis.
- [x] Add nemeses for process kill/restart and pause/resume.
- [x] Add a membership grow/shrink nemesis.
- [x] Add a read, write, and compare-and-set workload.
- [x] Add linearizability checking with Knossos.
- [x] Exercise snapshot construction during ordinary workloads.
