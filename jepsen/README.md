# OpenRaft Jepsen Tests

This directory contains Jepsen black-box tests for OpenRaft. The tests target the runnable OpenRaft key-value example service, rather than OpenRaft's internal Rust APIs, so they can validate externally observable behavior through real client requests, process lifecycle, and network behavior.

These tests cover a different layer from OpenRaft's deterministic simulation tests. Simulation tests run inside a controlled Rust environment. Jepsen drives a running KV service through its external API and records a client history for later checking. Jepsen runs are not deterministic, and a Jepsen failure is not expected to replay directly in the simulation harness.

## Organization

This is a standalone Clojure/Leiningen project inside the OpenRaft repository. It is not part of the Cargo workspace and should not be required for normal Rust builds or tests.

The Docker test environment separates the Jepsen control plane from the OpenRaft data plane:

```text
                         control container
                         Jepsen + Leiningen
                                  |
          SSH: setup / start / stop / logs
                                  |
        +-------------------------+-------------------------+
        |                         |                         |
        v                         v                         v
+---------------+         +---------------+         +---------------+
| n1 db node    |         | n2 db node    |         | n3 db node    |
| raft-kv-rocks |         | raft-kv-rocks |         | raft-kv-rocks |
+-------+-------+         +-------+-------+         +-------+-------+
        |                         |                         |
        | app_http API            | app_http API            | app_http API
        | same endpoints          | same endpoints          | same endpoints
        |                         |                         |
        +----------- Raft RPC: /append /vote /snapshot -----+
```

The control container is not an OpenRaft member. It runs the Jepsen process, controls the db nodes over SSH, and sends client operations to the KV service API. Every db node exposes the same `app_http` endpoints, and the leader-aware client may contact any of them. The OpenRaft cluster itself runs on `n1`, `n2`, and `n3`; those nodes communicate with each other over the Raft RPC port.

The intended layout is:

```text
jepsen/
  Makefile
  project.clj
  README.md
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
    workload.clj
```

The `jepsen.openraft` namespace contains the OpenRaft-specific Jepsen code:

- `cli.clj`: command-line entry point.
- `client.clj`: HTTP client for the OpenRaft KV example APIs.
- `db.clj`: Jepsen DB lifecycle for starting and stopping OpenRaft nodes.
- `cluster.clj`: cluster bootstrap helpers.
- `workload.clj`: generators and checkers for client operations.

## Running

### Prerequisites

- Docker with Compose support

### Run The Harness

From the repository root:

```bash
# Build images, start containers, and run the linearizability test.
$ make -C jepsen jepsen

# Generate the local Docker SSH key and build the Jepsen images.
$ make -C jepsen build

# Start or recreate the Jepsen containers.
$ make -C jepsen up

# Run the linearizability test against the running containers.
$ make -C jepsen test

# Stop and remove the Jepsen containers.
$ make -C jepsen down
```

This starts three Docker node containers, then runs the Jepsen control process from the control container. The test bootstraps a three-node cluster and checks a concurrent mix of linearizable reads, writes, and compare-and-set operations with Knossos.

## TODO

- [x] Add the Leiningen project definition and CLI skeleton.
- [x] Add Docker-based Jepsen control and node containers.
- [x] Add a multi-stage node image for the RocksDB KV example.
- [x] Add an HTTP client for the OpenRaft KV APIs.
- [x] Add Jepsen process lifecycle management for OpenRaft nodes.
- [x] Bootstrap a three-node OpenRaft cluster.
- [ ] Record acknowledged write, read, and info operation counts in each run.
- [ ] Add nemeses for network partitions and process kill/restart.
- [x] Add a read, write, and compare-and-set workload.
- [x] Add linearizability checking with Knossos.
- [ ] Add snapshot pressure and membership churn workloads.
