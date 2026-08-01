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
- `nemesis/process.clj`: quorum-safe process crashes and restarts.
- `workload.clj`: generators and checkers for client operations.

## Running

### Prerequisites

- Docker with Compose support

### Run The Harness

From the repository root:

```bash
# Format and lint the dedicated Rust test application.
$ make -C jepsen app-lint

# Build images, start containers, then run unit and linearizability tests.
$ make -C jepsen jepsen

# Generate the local Docker SSH key and build the Jepsen images.
$ make -C jepsen build

# Start or recreate the Jepsen containers.
$ make -C jepsen up

# Run the linearizability test against the running containers.
$ make -C jepsen test

# Run the process crash/restart test instead of the default partition test.
$ make -C jepsen test NEMESIS=process

# Run the membership change test.
$ make -C jepsen test NEMESIS=membership

# Reuse a recorded seed for Jepsen random choices.
$ make -C jepsen test NEMESIS=partition SEED=123456

# Stop and remove the Jepsen containers.
$ make -C jepsen down
```

The harness uses each Jepsen node's container hostname, such as `n1`, directly
as its OpenRaft node ID.

This starts the five-node Docker environment, then runs the Jepsen control
process from the control container. Every test checks a concurrent mix of
linearizable reads, writes, and compare-and-set operations with Knossos. While
the partition workload runs, Jepsen alternates between partitions where the
current leader is in the majority and in the minority. Each partition lasts 10
seconds. A run is valid only if both modes occur, every node agrees on a leader
after the final heal, client operations continue during recovery, and the final
recovery write and read succeed.

The process nemesis reads the effective voter configs from OpenRaft metrics and
randomly stops a non-empty voter subset whose survivors still form a quorum. It
supports both stable and joint membership, covers leader and follower-only
crashes, and waits for every stopped node to rejoin after restart.

The membership nemesis starts with a shrink and grow for deterministic coverage,
then randomly mixes additional membership changes. Removed nodes are stopped
and wiped only after the new voter set is committed. The final recovery restores
all five nodes as voters and waits for every node to agree on a leader.

Every run records a `:seed` in its stored Jepsen test data. Supplying that seed
again repeats choices made through `jepsen.random`, including the random node
selection used by the partition, process, and membership nemeses. It does not
make the whole run deterministic: client operation mixing and timing use
separate generator randomness, and thread, network, and election timing can
still differ. The recorded history and Nemesis operations are therefore the
authoritative account of the fault schedule; the seed is only an aid for
rerunning similar conditions.

## TODO

- [x] Add the Leiningen project definition and CLI skeleton.
- [x] Add Docker-based Jepsen control and node containers.
- [x] Add a multi-stage node image for the RocksDB KV test application.
- [x] Add an HTTP client for the OpenRaft KV APIs.
- [x] Add Jepsen process lifecycle management for OpenRaft nodes.
- [x] Bootstrap a five-node OpenRaft cluster.
- [x] Record phase-aware ok, fail, and info counts for read, write, and CAS.
- [x] Add a network partition nemesis.
- [x] Add nemeses for process kill/restart.
- [x] Add a membership grow/shrink nemesis.
- [x] Add a read, write, and compare-and-set workload.
- [x] Add linearizability checking with Knossos.
- [ ] Add snapshot pressure workloads.
