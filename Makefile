all: test defensive_test send_delay_test check_all

check_all: lint doc unused_dep typos detsim

compile:
	cargo test --lib
	cargo test --test '*'
	cargo test --features single-threaded --lib

# Read-only completion gate: it never writes to the worktree, so a
# human-reviewed diff stays exactly as reviewed. `make fix` is the mutating
# counterpart.
verify: fmt_check clippy docs-check
	cargo test --lib
	cargo test --test '*'
	# cargo test --features single-threaded --lib
	# test result in different output on CI are ignored and only run locally
	cargo test -p openraft-macros -- --ignored

basic_check: verify

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
	cargo test --manifest-path examples/dir-transfer/Cargo.toml
	cargo test --manifest-path examples/log-mem/Cargo.toml
	cargo test --manifest-path examples/log-rocks/Cargo.toml
	cargo test --manifest-path examples/log-wal/Cargo.toml
	cargo test --manifest-path examples/raft-kv-memstore/Cargo.toml
	cargo test --manifest-path examples/raft-kv-memstore-grpc/Cargo.toml
	cargo test --manifest-path examples/raft-kv-memstore-network-v1/Cargo.toml
	cargo test --manifest-path examples/raft-kv-memstore-opendal-snapshot-data/Cargo.toml
	cargo test --manifest-path examples/raft-kv-memstore-single-threaded/Cargo.toml
	cargo test --manifest-path examples/raft-kv-rocksdb/Cargo.toml
	cargo test --manifest-path examples/sm-rocks/Cargo.toml
	cargo test --manifest-path examples/multi-raft-kv/Cargo.toml

bench:
	cargo bench --features bench -p openraft

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

fmt_check:
	$(MAKE) fmt FMT_ARGS='-- --check'

# Apply every automatic rewrite: Clippy fixes first, then typo fixes, then
# formatting last so it also formats what the two fixers produced.
fix:
	cargo clippy --no-deps --all-targets --fix --allow-dirty --allow-staged
	$(MAKE) typos
	$(MAKE) fmt

doc:
	make -C openraft/src/docs/faq
	make -C openraft/src/docs/feature_flags
	RUSTDOCFLAGS="-D warnings" cargo doc --document-private-items --all --no-deps

# Read-only counterpart of `doc`: it builds the documentation, runs the
# doctests, and checks that repository links still resolve, without
# regenerating any file. The CI lint job runs this same target.
docs-check:
	RUSTDOCFLAGS="-D warnings" cargo doc --document-private-items --all --no-deps
	cargo test --doc --all
	./scripts/check-doc-links.py

check_missing_doc:
	# Warn about missing doc for public API
	RUSTDOCFLAGS="-W missing_docs" cargo doc --all --no-deps

guide:
	mdbook build
	@echo "doc is built in:"
	@echo "./guide/book/index.html"

detsim:
	cd tests-turmoil && cargo run --bin fuzz -- --iterations 5 --max-steps 10000

# Extra arguments passed to every `cargo fmt` below. Empty rewrites the files;
# `fmt_check` overrides it with `-- --check` to report without rewriting.
FMT_ARGS ?=

fmt:
	cargo fmt $(FMT_ARGS)
	cargo fmt --manifest-path multiraft/Cargo.toml $(FMT_ARGS)
	cargo fmt --manifest-path rt-compio/Cargo.toml $(FMT_ARGS)
	cargo fmt --manifest-path rt-monoio/Cargo.toml $(FMT_ARGS)
	cargo fmt --manifest-path rt-tokio/Cargo.toml $(FMT_ARGS)
	cargo fmt --manifest-path metrics-otel/Cargo.toml $(FMT_ARGS)
	cargo fmt --manifest-path benchmarks/minimal/Cargo.toml $(FMT_ARGS)
	cargo fmt --manifest-path examples/app-http/Cargo.toml $(FMT_ARGS)
	cargo fmt --manifest-path examples/network-v1-http/Cargo.toml $(FMT_ARGS)
	cargo fmt --manifest-path examples/network-v2-http/Cargo.toml $(FMT_ARGS)
	cargo fmt --manifest-path examples/dir-transfer/Cargo.toml $(FMT_ARGS)
	cargo fmt --manifest-path examples/log-mem/Cargo.toml $(FMT_ARGS)
	cargo fmt --manifest-path examples/log-rocks/Cargo.toml $(FMT_ARGS)
	cargo fmt --manifest-path examples/log-wal/Cargo.toml $(FMT_ARGS)
	cargo fmt --manifest-path examples/sm-mem/Cargo.toml $(FMT_ARGS)
	cargo fmt --manifest-path examples/sm-rocks/Cargo.toml $(FMT_ARGS)
	cargo fmt --manifest-path examples/types-kv/Cargo.toml $(FMT_ARGS)
	cargo fmt --manifest-path examples/raft-kv-memstore-grpc/Cargo.toml $(FMT_ARGS)
	cargo fmt --manifest-path examples/raft-kv-memstore-network-v1/Cargo.toml $(FMT_ARGS)
	cargo fmt --manifest-path examples/raft-kv-memstore-opendal-snapshot-data/Cargo.toml $(FMT_ARGS)
	cargo fmt --manifest-path examples/raft-kv-memstore-single-threaded/Cargo.toml $(FMT_ARGS)
	cargo fmt --manifest-path examples/raft-kv-memstore/Cargo.toml $(FMT_ARGS)
	cargo fmt --manifest-path examples/raft-kv-rocksdb/Cargo.toml $(FMT_ARGS)
	cargo fmt --manifest-path examples/multi-raft-kv/Cargo.toml $(FMT_ARGS)
	cargo fmt --manifest-path tests-turmoil/Cargo.toml $(FMT_ARGS)
	cargo fmt --manifest-path jepsen/openraft-test-app/Cargo.toml $(FMT_ARGS)

clippy:
	@# The three workspace clippy runs mirror the CI lint job
	@# (.github/workflows/ci.yaml); keep them in sync so `make lint` fails
	@# exactly where CI would, feature unification included.
	cargo clippy --no-deps --workspace --all-targets -- -D warnings
	cargo clippy --no-deps --workspace --all-targets --features "bt,serde,bench,compat" -- -D warnings
	cargo clippy --no-deps --workspace --all-targets --features "metrics-logids,serde" -- -D warnings
	cargo clippy --no-deps --manifest-path multiraft/Cargo.toml                                       --all-targets -- -D warnings
	cargo clippy --no-deps --manifest-path rt-compio/Cargo.toml                                       --all-targets -- -D warnings
	cargo clippy --no-deps --manifest-path rt-monoio/Cargo.toml                                       --all-targets -- -D warnings
	cargo clippy --no-deps --manifest-path rt-tokio/Cargo.toml                                        --all-targets -- -D warnings
	cargo clippy --no-deps --manifest-path metrics-otel/Cargo.toml                                    --all-targets -- -D warnings
	cargo clippy --no-deps --manifest-path benchmarks/minimal/Cargo.toml                               --all-targets -- -D warnings
	cargo clippy --no-deps --manifest-path examples/app-http/Cargo.toml                               --all-targets -- -D warnings
	cargo clippy --no-deps --manifest-path examples/network-v1-http/Cargo.toml                         --all-targets -- -D warnings
	cargo clippy --no-deps --manifest-path examples/network-v2-http/Cargo.toml                         --all-targets -- -D warnings
	cargo clippy --no-deps --manifest-path examples/dir-transfer/Cargo.toml                           --all-targets -- -D warnings
	cargo clippy --no-deps --manifest-path examples/log-mem/Cargo.toml                                --all-targets -- -D warnings
	cargo clippy --no-deps --manifest-path examples/log-rocks/Cargo.toml                              --all-targets -- -D warnings
	cargo clippy --no-deps --manifest-path examples/log-wal/Cargo.toml                                --all-targets -- -D warnings
	cargo clippy --no-deps --manifest-path examples/sm-mem/Cargo.toml                                --all-targets -- -D warnings
	cargo clippy --no-deps --manifest-path examples/sm-rocks/Cargo.toml                                --all-targets -- -D warnings
	cargo clippy --no-deps --manifest-path examples/types-kv/Cargo.toml                               --all-targets -- -D warnings
	cargo clippy --no-deps --manifest-path examples/raft-kv-memstore-grpc/Cargo.toml                  --all-targets -- -D warnings
	cargo clippy --no-deps --manifest-path examples/raft-kv-memstore-network-v1/Cargo.toml            --all-targets -- -D warnings
	cargo clippy --no-deps --manifest-path examples/raft-kv-memstore-opendal-snapshot-data/Cargo.toml --all-targets -- -D warnings
	cargo clippy --no-deps --manifest-path examples/raft-kv-memstore-single-threaded/Cargo.toml       --all-targets -- -D warnings
	cargo clippy --no-deps --manifest-path examples/raft-kv-memstore/Cargo.toml                       --all-targets -- -D warnings
	cargo clippy --no-deps --manifest-path examples/raft-kv-rocksdb/Cargo.toml                        --all-targets -- -D warnings
	cargo clippy --no-deps --manifest-path examples/multi-raft-kv/Cargo.toml                          --all-targets -- -D warnings
	@# Run from inside tests-turmoil so cargo reads its .cargo/config.toml,
	@# which supplies `--cfg tokio_unstable`; `--manifest-path` from the repo
	@# root would not pick it up.
	cd tests-turmoil && cargo clippy --no-deps --all-targets -- -D warnings
	cargo clippy --no-deps --manifest-path jepsen/openraft-test-app/Cargo.toml                        --all-targets -- -D warnings
	@# Bug: clippy --all-targets reports false warning about unused dep in
	@# `[dev-dependencies]`:
	@# https://github.com/rust-lang/rust/issues/72686#issuecomment-635539688
	@# Thus we only check unused deps for lib
	RUSTFLAGS=-Wunused-crate-dependencies cargo clippy --no-deps  --lib -- -D warnings

lint: fmt clippy

unused_dep:
	cargo machete
	cargo machete examples/app-http
	cargo machete examples/raft-kv-memstore
	cargo machete examples/raft-kv-rocksdb
	cargo machete examples/raft-kv-memstore-grpc
	cargo machete examples/raft-kv-memstore-single-threaded
	cargo machete examples/raft-kv-memstore-opendal-snapshot-data
	cargo machete examples/raft-kv-memstore-network-v1
	cargo machete examples/multi-raft-kv
	cargo machete examples/sm-rocks
	cargo machete examples/log-rocks
	cargo machete examples/log-wal
	cargo machete examples/dir-transfer
	cargo machete multiraft
	cargo machete rt-compio
	cargo machete rt-monoio
	cargo machete rt-tokio

typos:
	# cargo install typos-cli
	typos --write-changes openraft/ tests/ stores/memstore/ stores/memstore-custom-node-id/ stores/rocksstore examples/raft-kv-memstore/ examples/raft-kv-rocksdb/
	#typos --write-changes --exclude change-log/ --exclude change-log.md --exclude derived-from-async-raft.md
	# typos

check:
	RUSTFLAGS="-D warnings" cargo check
	RUSTFLAGS="-D warnings" cargo check --manifest-path multiraft/Cargo.toml
	RUSTFLAGS="-D warnings" cargo check --manifest-path rt-compio/Cargo.toml
	RUSTFLAGS="-D warnings" cargo check --manifest-path rt-monoio/Cargo.toml
	RUSTFLAGS="-D warnings" cargo check --manifest-path rt-tokio/Cargo.toml
	RUSTFLAGS="-D warnings" cargo check --manifest-path metrics-otel/Cargo.toml
	RUSTFLAGS="-D warnings" cargo check --manifest-path benchmarks/minimal/Cargo.toml
	RUSTFLAGS="-D warnings" cargo check --manifest-path examples/app-http/Cargo.toml
	RUSTFLAGS="-D warnings" cargo check --manifest-path examples/network-v1-http/Cargo.toml
	RUSTFLAGS="-D warnings" cargo check --manifest-path examples/network-v2-http/Cargo.toml
	RUSTFLAGS="-D warnings" cargo check --manifest-path examples/dir-transfer/Cargo.toml
	RUSTFLAGS="-D warnings" cargo check --manifest-path examples/log-mem/Cargo.toml
	RUSTFLAGS="-D warnings" cargo check --manifest-path examples/log-rocks/Cargo.toml
	RUSTFLAGS="-D warnings" cargo check --manifest-path examples/log-wal/Cargo.toml
	RUSTFLAGS="-D warnings" cargo check --manifest-path examples/sm-mem/Cargo.toml
	RUSTFLAGS="-D warnings" cargo check --manifest-path examples/sm-rocks/Cargo.toml
	RUSTFLAGS="-D warnings" cargo check --manifest-path examples/types-kv/Cargo.toml
	RUSTFLAGS="-D warnings" cargo check --manifest-path examples/raft-kv-memstore-grpc/Cargo.toml
	RUSTFLAGS="-D warnings" cargo check --manifest-path examples/raft-kv-memstore-network-v1/Cargo.toml
	RUSTFLAGS="-D warnings" cargo check --manifest-path examples/raft-kv-memstore-opendal-snapshot-data/Cargo.toml
	RUSTFLAGS="-D warnings" cargo check --manifest-path examples/raft-kv-memstore-single-threaded/Cargo.toml
	RUSTFLAGS="-D warnings" cargo check --manifest-path examples/raft-kv-memstore/Cargo.toml
	RUSTFLAGS="-D warnings" cargo check --manifest-path examples/raft-kv-rocksdb/Cargo.toml
	RUSTFLAGS="-D warnings" cargo check --manifest-path examples/multi-raft-kv/Cargo.toml

clean:
	cargo clean
	cargo clean --manifest-path multiraft/Cargo.toml
	cargo clean --manifest-path rt-compio/Cargo.toml
	cargo clean --manifest-path rt-monoio/Cargo.toml
	cargo clean --manifest-path rt-tokio/Cargo.toml
	cargo clean --manifest-path metrics-otel/Cargo.toml
	cargo clean --manifest-path benchmarks/minimal/Cargo.toml
	cargo clean --manifest-path tests-turmoil/Cargo.toml
	cargo clean --manifest-path examples/app-http/Cargo.toml
	cargo clean --manifest-path examples/network-v1-http/Cargo.toml
	cargo clean --manifest-path examples/network-v2-http/Cargo.toml
	cargo clean --manifest-path examples/dir-transfer/Cargo.toml
	cargo clean --manifest-path examples/log-mem/Cargo.toml
	cargo clean --manifest-path examples/log-rocks/Cargo.toml
	cargo clean --manifest-path examples/log-wal/Cargo.toml
	cargo clean --manifest-path examples/sm-mem/Cargo.toml
	cargo clean --manifest-path examples/sm-rocks/Cargo.toml
	cargo clean --manifest-path examples/types-kv/Cargo.toml
	cargo clean --manifest-path examples/raft-kv-memstore-grpc/Cargo.toml
	cargo clean --manifest-path examples/raft-kv-memstore-network-v1/Cargo.toml
	cargo clean --manifest-path examples/raft-kv-memstore-opendal-snapshot-data/Cargo.toml
	cargo clean --manifest-path examples/raft-kv-memstore-single-threaded/Cargo.toml
	cargo clean --manifest-path examples/raft-kv-memstore/Cargo.toml
	cargo clean --manifest-path examples/raft-kv-rocksdb/Cargo.toml
	cargo clean --manifest-path examples/multi-raft-kv/Cargo.toml
	cargo clean --manifest-path jepsen/openraft-test-app/Cargo.toml
	rm -rf tests/_log

.PHONY: test verify basic_check fmt fmt_check fix clippy lint clean doc docs-check guide detsim typos
