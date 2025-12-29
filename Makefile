all: test defensive_test send_delay_test check_all

check_all: lint fmt doc unused_dep typos

compile:
	cargo test --lib
	cargo test --test '*'
	cargo test --features single-threaded --lib

basic_check:
	cargo fmt
	cargo clippy --no-deps --all-targets --fix --allow-dirty --allow-staged
	cargo test --lib
	cargo test --test '*'
	# cargo test --features single-threaded --lib
	cargo clippy --no-deps --all-targets -- -D warnings
	RUSTDOCFLAGS="-D warnings" cargo doc --document-private-items --all --no-deps
	# test result in different output on CI are ignored and only run locally
	cargo test -p openraft-macros -- --ignored

defensive_test:
	OPENRAFT_STORE_DEFENSIVE=on cargo test

send_delay_test:
	OPENRAFT_NETWORK_SEND_DELAY=30 cargo test

test:
	cargo test
	cargo test --features bt
	cargo test --features serde
	# only crate `tests` has single-term-leader feature
	cargo test --features single-term-leader -p tests
	# multiraft crate tests
	cargo test --manifest-path multiraft/Cargo.toml
	$(MAKE) test-examples

check-parallel:
	./scripts/check-parallel.sh


test-examples:
	cargo test --manifest-path examples/log-mem/Cargo.toml
	cargo test --manifest-path examples/raft-kv-memstore/Cargo.toml
	cargo test --manifest-path examples/raft-kv-memstore-grpc/Cargo.toml
	cargo test --manifest-path examples/raft-kv-memstore-network-v2/Cargo.toml
	cargo test --manifest-path examples/raft-kv-memstore-opendal-snapshot-data/Cargo.toml
	cargo test --manifest-path examples/raft-kv-memstore-single-threaded/Cargo.toml
	cargo test --manifest-path examples/raft-kv-rocksdb/Cargo.toml
	cargo test --manifest-path examples/rocksstore/Cargo.toml
	cargo test --manifest-path examples/multi-raft-kv/Cargo.toml

bench:
	cargo bench --features bench

# Set TOKIO_CONSOLE=1 to enable tokio-console support
# Set FLAMEGRAPH=1 to enable flamegraph profiling
# Example: TOKIO_CONSOLE=1 make bench_cluster_of_3
comma := ,
BENCH_FEATURES := $(if $(TOKIO_CONSOLE),tokio-console,)$(if $(FLAMEGRAPH),$(if $(TOKIO_CONSOLE),$(comma))flamegraph,)
BENCH_FEATURES_FLAG := $(if $(BENCH_FEATURES),--features $(BENCH_FEATURES),)
BENCH_RUSTFLAGS := $(if $(TOKIO_CONSOLE),RUSTFLAGS="--cfg tokio_unstable",)

bench_cluster_of_1:
	$(BENCH_RUSTFLAGS) cargo run --manifest-path benchmarks/minimal/Cargo.toml --release --bin bench $(BENCH_FEATURES_FLAG) -- -m 1

bench_cluster_of_3:
	$(BENCH_RUSTFLAGS) cargo run --manifest-path benchmarks/minimal/Cargo.toml --release --bin bench $(BENCH_FEATURES_FLAG) -- -m 3

bench_cluster_of_5:
	$(BENCH_RUSTFLAGS) cargo run --manifest-path benchmarks/minimal/Cargo.toml --release --bin bench $(BENCH_FEATURES_FLAG) -- -m 5

fmt:
	cargo fmt

fix:
	cargo fix --allow-staged

doc:
	make -C openraft/src/docs/faq
	make -C openraft/src/docs/feature_flags
	RUSTDOCFLAGS="-D warnings" cargo doc --document-private-items --all --no-deps

check_missing_doc:
	# Warn about missing doc for public API
	RUSTDOCFLAGS="-W missing_docs" cargo doc --all --no-deps

guide:
	mdbook build
	@echo "doc is built in:"
	@echo "./guide/book/index.html"

lint:
	cargo fmt
	cargo fmt --manifest-path multiraft/Cargo.toml
	cargo fmt --manifest-path rt-compio/Cargo.toml
	cargo fmt --manifest-path rt-monoio/Cargo.toml
	cargo fmt --manifest-path rt-tokio/Cargo.toml
	cargo fmt --manifest-path examples/log-mem/Cargo.toml
	cargo fmt --manifest-path examples/sm-mem/Cargo.toml
	cargo fmt --manifest-path examples/raft-kv-memstore-grpc/Cargo.toml
	cargo fmt --manifest-path examples/raft-kv-memstore-network-v2/Cargo.toml
	cargo fmt --manifest-path examples/raft-kv-memstore-opendal-snapshot-data/Cargo.toml
	cargo fmt --manifest-path examples/raft-kv-memstore-single-threaded/Cargo.toml
	cargo fmt --manifest-path examples/raft-kv-memstore/Cargo.toml
	cargo fmt --manifest-path examples/raft-kv-rocksdb/Cargo.toml
	cargo fmt --manifest-path examples/multi-raft-kv/Cargo.toml
	cargo clippy --no-deps --all-targets -- -D warnings
	cargo clippy --no-deps --manifest-path multiraft/Cargo.toml                                       --all-targets -- -D warnings
	cargo clippy --no-deps --manifest-path rt-compio/Cargo.toml                                       --all-targets -- -D warnings
	cargo clippy --no-deps --manifest-path rt-monoio/Cargo.toml                                       --all-targets -- -D warnings
	cargo clippy --no-deps --manifest-path rt-tokio/Cargo.toml                                        --all-targets -- -D warnings
	cargo clippy --no-deps --manifest-path examples/log-mem/Cargo.toml                                --all-targets -- -D warnings
	cargo clippy --no-deps --manifest-path examples/raft-kv-memstore-grpc/Cargo.toml                  --all-targets -- -D warnings
	cargo clippy --no-deps --manifest-path examples/raft-kv-memstore-network-v2/Cargo.toml            --all-targets -- -D warnings
	cargo clippy --no-deps --manifest-path examples/raft-kv-memstore-opendal-snapshot-data/Cargo.toml --all-targets -- -D warnings
	cargo clippy --no-deps --manifest-path examples/raft-kv-memstore-single-threaded/Cargo.toml       --all-targets -- -D warnings
	cargo clippy --no-deps --manifest-path examples/raft-kv-memstore/Cargo.toml                       --all-targets -- -D warnings
	cargo clippy --no-deps --manifest-path examples/raft-kv-rocksdb/Cargo.toml                        --all-targets -- -D warnings
	cargo clippy --no-deps --manifest-path examples/multi-raft-kv/Cargo.toml                          --all-targets -- -D warnings
	# Bug: clippy --all-targets reports false warning about unused dep in
	# `[dev-dependencies]`:
	# https://github.com/rust-lang/rust/issues/72686#issuecomment-635539688
	# Thus we only check unused deps for lib
	RUSTFLAGS=-Wunused-crate-dependencies cargo clippy --no-deps  --lib -- -D warnings

unused_dep:
	cargo machete
	cargo machete examples/raft-kv-memstore
	cargo machete examples/raft-kv-rocksdb
	cargo machete examples/raft-kv-memstore-grpc
	cargo machete examples/raft-kv-memstore-single-threaded
	cargo machete examples/raft-kv-memstore-opendal-snapshot-data
	cargo machete examples/raft-kv-memstore-network-v2
	cargo machete examples/multi-raft-kv
	cargo machete examples/rocksstore
	cargo machete multiraft
	cargo machete rt-compio
	cargo machete rt-monoio
	cargo machete rt-tokio

typos:
	# cargo install typos-cli
	typos --write-changes openraft/ tests/ stores/memstore/ stores/rocksstore examples/raft-kv-memstore/ examples/raft-kv-rocksdb/
	#typos --write-changes --exclude change-log/ --exclude change-log.md --exclude derived-from-async-raft.md
	# typos

check:
	RUSTFLAGS="-D warnings" cargo check
	RUSTFLAGS="-D warnings" cargo check --manifest-path multiraft/Cargo.toml
	RUSTFLAGS="-D warnings" cargo check --manifest-path rt-compio/Cargo.toml
	RUSTFLAGS="-D warnings" cargo check --manifest-path rt-monoio/Cargo.toml
	RUSTFLAGS="-D warnings" cargo check --manifest-path rt-tokio/Cargo.toml
	RUSTFLAGS="-D warnings" cargo check --manifest-path benchmarks/minimal/Cargo.toml
	RUSTFLAGS="-D warnings" cargo check --manifest-path examples/client-http/Cargo.toml
	RUSTFLAGS="-D warnings" cargo check --manifest-path examples/network-v1-http/Cargo.toml
	RUSTFLAGS="-D warnings" cargo check --manifest-path examples/log-mem/Cargo.toml
	RUSTFLAGS="-D warnings" cargo check --manifest-path examples/sm-mem/Cargo.toml
	RUSTFLAGS="-D warnings" cargo check --manifest-path examples/types-kv/Cargo.toml
	RUSTFLAGS="-D warnings" cargo check --manifest-path examples/raft-kv-memstore-grpc/Cargo.toml
	RUSTFLAGS="-D warnings" cargo check --manifest-path examples/raft-kv-memstore-network-v2/Cargo.toml
	RUSTFLAGS="-D warnings" cargo check --manifest-path examples/raft-kv-memstore-opendal-snapshot-data/Cargo.toml
	RUSTFLAGS="-D warnings" cargo check --manifest-path examples/raft-kv-memstore-single-threaded/Cargo.toml
	RUSTFLAGS="-D warnings" cargo check --manifest-path examples/raft-kv-memstore/Cargo.toml
	RUSTFLAGS="-D warnings" cargo check --manifest-path examples/raft-kv-rocksdb/Cargo.toml
	RUSTFLAGS="-D warnings" cargo check --manifest-path examples/rocksstore/Cargo.toml
	RUSTFLAGS="-D warnings" cargo check --manifest-path examples/multi-raft-kv/Cargo.toml

clean:
	cargo clean
	cargo clean --manifest-path multiraft/Cargo.toml
	cargo clean --manifest-path rt-compio/Cargo.toml
	cargo clean --manifest-path rt-monoio/Cargo.toml
	cargo clean --manifest-path rt-tokio/Cargo.toml
	cargo clean --manifest-path benchmarks/minimal/Cargo.toml
	cargo clean --manifest-path examples/client-http/Cargo.toml
	cargo clean --manifest-path examples/network-v1-http/Cargo.toml
	cargo clean --manifest-path examples/log-mem/Cargo.toml
	cargo clean --manifest-path examples/raft-kv-memstore-grpc/Cargo.toml
	cargo clean --manifest-path examples/raft-kv-memstore-network-v2/Cargo.toml
	cargo clean --manifest-path examples/raft-kv-memstore-opendal-snapshot-data/Cargo.toml
	cargo clean --manifest-path examples/raft-kv-memstore-single-threaded/Cargo.toml
	cargo clean --manifest-path examples/raft-kv-memstore/Cargo.toml
	cargo clean --manifest-path examples/raft-kv-rocksdb/Cargo.toml
	cargo clean --manifest-path examples/rocksstore/Cargo.toml
	cargo clean --manifest-path examples/multi-raft-kv/Cargo.toml

.PHONY: test fmt lint clean doc guide
