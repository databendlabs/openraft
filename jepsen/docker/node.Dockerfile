# librocksdb-sys bundles RocksDB 8.10 and compiles its C++ from source, which
# took 7m39s of the 8m13s image build. Link the system librocksdb instead;
# ROCKSDB_INCLUDE_DIR points bindgen at the matching headers so the generated
# bindings match the installed lib. Both stages use Ubuntu 24.04 because its
# librocksdb is the version already proven against this crate by the
# `examples` job in .github/workflows/ci.yaml.
FROM ubuntu:24.04 AS builder

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      build-essential \
      ca-certificates \
      clang \
      cmake \
      curl \
      libclang-dev \
      librocksdb-dev \
      libssl-dev \
      pkg-config \
      protobuf-compiler \
 && rm -rf /var/lib/apt/lists/*

# No default toolchain: cargo runs under /openraft, so the root rust-toolchain
# pin drives which toolchain rustup installs on first use.
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
      | sh -s -- -y --profile minimal --default-toolchain none
ENV PATH=/root/.cargo/bin:$PATH

ENV ROCKSDB_LIB_DIR=/usr/lib/x86_64-linux-gnu \
    ROCKSDB_INCLUDE_DIR=/usr/include

WORKDIR /openraft
COPY . .

RUN cargo build --release \
      --manifest-path jepsen/openraft-test-app/Cargo.toml

FROM ubuntu:24.04

# The binary links librocksdb dynamically, so the runtime image needs it too.
# Use the -dev package: its name is stable across RocksDB version bumps, while
# the versioned runtime package name is not.
#
# `jepsen.openraft.db` controls the test app through the process tools below
# and depends on three of their behaviors. `db_test.clj` redefines `c/exec`,
# so it covers the Clojure decision logic and never these behaviors; the only
# real check is the post-merge `process` job in .github/workflows/jepsen.yml,
# which reports a break as a Jepsen run failure rather than a named test.
# Re-verify all three when moving off Ubuntu 24.04:
#
# - `pgrep --ignore-ancestors` needs procps-ng 4.0 or later; this image has
#   procps 4.0.4. Under procps 3.3.17, as in Debian bullseye, every process
#   probe exits non-zero.
# - psmisc's `killall` reports a missing target as `<name>: no process found`.
#   `no-such-process-race?` matches that exact wording, so a reworded message
#   turns a benign exit race into a Harness failure.
# - `start-stop-daemon --oknodo`, which the base image provides rather than
#   the list below, converts only the nothing-to-do case to exit 0, so a real
#   start failure still exits non-zero.
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      ca-certificates \
      iproute2 \
      iptables \
      libfaketime \
      libgcc-s1 \
      librocksdb-dev \
      libstdc++6 \
      netcat-openbsd \
      openssh-server \
      procps \
      psmisc \
      sudo \
 && rm -rf /var/lib/apt/lists/*

# Use one architecture-independent path in the application launch environment.
RUN faketime_lib="$(find /usr/lib -path '*/faketime/libfaketime.so.1' -print -quit)" \
 && test -n "$faketime_lib" \
 && ln -s "$faketime_lib" /usr/local/lib/libfaketime.so.1

RUN mkdir -p /run/sshd /var/lib/openraft /var/log/openraft /root/.ssh \
 && chmod 700 /root/.ssh \
 && echo "root:root" | chpasswd \
 && printf "\nPermitRootLogin prohibit-password\nPasswordAuthentication no\nPubkeyAuthentication yes\n" >> /etc/ssh/sshd_config

COPY jepsen/docker/ssh/openraft-jepsen.pub /root/.ssh/authorized_keys
RUN chmod 600 /root/.ssh/authorized_keys

COPY --from=builder \
  /openraft/jepsen/openraft-test-app/target/release/openraft-jepsen-app \
  /usr/local/bin/openraft-jepsen-app

EXPOSE 22 21001 22001

CMD ["/usr/sbin/sshd", "-D", "-e"]
