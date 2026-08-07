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
    cli.clj
    client.clj
    db.clj
    cluster.clj
    nemesis/
      membership.clj
      partition.clj
      process.clj
    workload.clj
```

The `openraft-test-app` crate is derived from the RocksDB example but belongs to
the Jepsen harness. It uses string node IDs so each Docker hostname can also be
the corresponding OpenRaft node ID. Test-specific application changes can be
made there without increasing the complexity of the general-purpose example.

The `jepsen.openraft` namespace contains the OpenRaft-specific Jepsen code:

- `cli.clj`: command-line entry point.
- `client.clj`: HTTP client for the OpenRaft KV example APIs.
- `db.clj`: Jepsen DB lifecycle for starting and stopping OpenRaft nodes.
- `cluster.clj`: cluster bootstrap helpers.
- `nemesis/membership.clj`: membership growth, shrink, and final restoration.
- `nemesis/partition.clj`: leader-aware network partition faults and recovery.
- `nemesis/process.clj`: quorum-safe process kill/restart and pause/resume faults.
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

## TODO

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
