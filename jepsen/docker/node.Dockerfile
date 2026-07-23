FROM rust:bookworm AS builder

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      build-essential \
      clang \
      cmake \
      libclang-dev \
      libssl-dev \
      pkg-config \
      protobuf-compiler \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /openraft
COPY . .

RUN cargo build --release \
      --manifest-path examples/raft-kv-rocksdb/Cargo.toml

FROM debian:bookworm-slim

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      ca-certificates \
      iproute2 \
      iptables \
      libgcc-s1 \
      libstdc++6 \
      netcat-openbsd \
      openssh-server \
      procps \
      psmisc \
      sudo \
 && rm -rf /var/lib/apt/lists/*

RUN mkdir -p /run/sshd /var/lib/openraft /var/log/openraft /root/.ssh \
 && chmod 700 /root/.ssh \
 && echo "root:root" | chpasswd \
 && printf "\nPermitRootLogin prohibit-password\nPasswordAuthentication no\nPubkeyAuthentication yes\n" >> /etc/ssh/sshd_config

COPY jepsen/docker/ssh/openraft-jepsen.pub /root/.ssh/authorized_keys
RUN chmod 600 /root/.ssh/authorized_keys

COPY --from=builder \
  /openraft/examples/raft-kv-rocksdb/target/release/raft-key-value-rocks \
  /usr/local/bin/raft-key-value-rocks

EXPOSE 22 21001 22001

CMD ["/usr/sbin/sshd", "-D", "-e"]
