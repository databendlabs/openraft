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
registers with Knossos. `NEMESIS` accepts a comma-separated subset when a
narrower combination is needed.

### Nemesis Design

A standalone Nemesis selects each target so the surviving voters still contain
a quorum in every effective voter configuration. This applies to both stable
and joint membership. A focused run can therefore exercise one fault class and
its recovery while retaining a component that can make progress.

The `chaos` profile composes individually quorum-safe faults without reserving
one common survivor quorum across fault classes. Their active intervals may
overlap and temporarily remove the global quorum. Safety and eventual recovery
remain mandatory in that case, but continuous availability does not.

#### Network Partition Nemesis

The partition Nemesis installs a hard network partition between two components.
It exercises two leader-aware cases:

- `leader-in-majority`: the current quorum-supported leader remains with a
  majority of the voters;
- `leader-in-minority`: the leader is isolated with a minority while the other
  component retains a quorum and can elect a replacement.

A focused partition run requires both cases to be installed and healed. If no
quorum-supported leader or safe partition target is observable, the operation
is skipped and retried later without counting as coverage.

#### Process Kill Nemesis

The process Nemesis reads the effective voter configurations from OpenRaft
metrics and randomly stops a non-empty voter subset whose survivors still form
a quorum. It exercises two target cases:

- `leader-killed`: the selected processes include the current
  quorum-supported leader, so the survivors must elect a replacement;
- `leader-survives`: only non-leader voters are selected, so the existing leader
  remains with a quorum.

The target set is fixed for one fault episode. Recovery restarts the selected
processes and waits for them to rejoin the healthy cluster.

#### Process Pause Nemesis

The pause Nemesis uses the same quorum-safe target selection as process kill,
but suspends the selected processes without terminating them. It covers:

- `leader-paused`: the selected set includes the supported leader;
- `leader-unpaused`: only non-leader voters are suspended.

Paused processes retain their memory and open TCP connections, so peers observe
unresponsive processes rather than closed connections. Resume operations target
every test node as an idempotent cleanup and record the complete resumed node
set in history.

#### Membership Nemesis

The membership Nemesis exercises two membership-change cases:

- `shrink`: remove a voter only when the resulting configuration retains a
  quorum;
- `grow`: add a non-voter back through learner initialization and committed
  membership change.

It starts with one shrink and grow for deterministic coverage, then randomly
mixes additional membership changes. Removed nodes are stopped and wiped only
after the new voter set is committed. Final recovery restores all five nodes as
voters and waits for every node to agree on a leader.

#### Packet Nemesis

The planned packet Nemesis models traffic that remains reachable but is
degraded. Hard packet drops that create a partition remain the responsibility
of the Network Partition Nemesis. Packet provides two mutually exclusive
modes:

- `slow`: 300 ms latency with 50 ms jitter;
- `flaky`: Jepsen's default 20% packet loss with 75% correlation.

The `slow` parameters are derived from the test application's timing
configuration. Its election timeout is approximately 299 ms and its heartbeat
interval is 50 ms. A 300 ms base delay with 50 ms jitter therefore spans roughly
250--350 ms: from about one heartbeat interval before the election threshold to
about one heartbeat interval after it. This deliberately exercises the boundary
where some messages arrive before an election timeout and others arrive after
it, without requiring every fault episode to trigger an election.

A focused run explicitly selects one mode. Within that mode it exercises two
quorum-safe target cases:

- `leader-included`: the fixed target set includes the quorum-supported leader;
- `leader-excluded`: the fixed target set contains only non-leader voters.

Targets are selected once per fault episode and do not follow a newly elected
leader. If no supported leader or safe target for the requested case is
observable, the operation is skipped and retried without counting as coverage.

Both modes use Jepsen's `shape!` wrapper around Linux `tc netem`. The first
version applies one mode in both directions across the selected target boundary,
at node-IP granularity and across DB-to-DB ports. It does not provide one-way,
per-RPC, or per-port shaping. Because both modes own the same root qdisc, they
are never active at the same time.

The same Packet package and checker are used in focused and chaos runs. Packet
coverage requires the selected mode and both leader-target cases to be installed
and later cleared successfully. It does not require an election, timeout, or
indeterminate mutation, because those observations depend on timing, TCP
retransmission, and kernel packet selection. Partial installation or cleanup is
a Harness failure; final recovery and teardown must attempt idempotent cleanup
on every node.

#### Chaos Composition

The default `chaos` profile independently schedules partition, process, pause,
membership, and, once implemented, packet faults. It composes the package,
generator, final recovery, and checker supplied by each Nemesis rather than
defining separate Chaos-only checker semantics. Packet chooses one of `slow` or
`flaky` for each independent Packet episode, and never overlaps those two modes
with each other. Different fault classes may remain active at the same time.

Every Jepsen node builds a snapshot after 100 newly committed logs by default.
The regular write workload therefore exercises snapshot construction during
short fault tests without a snapshot-specific Nemesis. Set
`SNAPSHOT_THRESHOLD` to override the threshold for a run.

After the fault schedule ends, Jepsen heals partitions, restarts killed
processes, resumes paused processes, restores membership, and then performs one
shared readiness check. Client operations continue while faults are active and
during recovery. Final recovery performs one write and one read per register;
every operation must succeed.

### Results and Stored Evidence

[Jepsen error-handling semantics](error-handling-semantics.md) defines the
Outcome and Harness-failure contract. The sections below explain how to read
the resulting checker verdicts and stored artifacts.

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

The `final-workload` checker separately rejects a run when its final reads or
writes fail. These modeled failures are not unexpected-SUT-response markers.

A coverage failure rejects the run but does not by itself identify an OpenRaft
failure or its root cause. For example, `:missing-modes` is written to
`results.edn`; the mode may be absent from `history.edn`, skipped by an outcome
such as `:no-supported-leader`, or blocked by a Harness failure recorded in
`jepsen.log`. Start with the nested checker result, inspect the corresponding
operations in `history.edn`, and then use `jepsen.log` for Harness diagnostics.

The top-level `:valid?` combines these independent verdicts and answers only
whether the entire run can be accepted. It does not by itself identify an
OpenRaft property violation or the state of the Harness.

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
