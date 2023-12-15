all: test defensive_test send_delay_test check_all

check_all: lint fmt doc unused_dep typos

defensive_test:
	OPENRAFT_STORE_DEFENSIVE=on cargo test

send_delay_test:
	OPENRAFT_NETWORK_SEND_DELAY=30 cargo test

test:
	cargo test
	cargo test --features bt
	cargo test --features serde
	cargo test --features single-term-leader
	cargo test --manifest-path examples/raft-kv-memstore/Cargo.toml
	cargo test --manifest-path examples/raft-kv-rocksdb/Cargo.toml

bench:
	cargo bench --features bench

bench_cluster_of_1:
	cargo test --manifest-path cluster_benchmark/Cargo.toml --test benchmark --release bench_cluster_of_1 -- --ignored --nocapture

bench_cluster_of_3:
	cargo test --manifest-path cluster_benchmark/Cargo.toml --test benchmark --release bench_cluster_of_3 -- --ignored --nocapture

bench_cluster_of_5:
	cargo test --manifest-path cluster_benchmark/Cargo.toml --test benchmark --release bench_cluster_of_5 -- --ignored --nocapture

fmt:
	cargo fmt

fix:
	cargo fix --allow-staged

doc:
	make -C openraft/src/docs/faq
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
	cargo fmt --manifest-path examples/raft-kv-memstore/Cargo.toml
	cargo fmt --manifest-path examples/raft-kv-rocksdb/Cargo.toml
	cargo clippy --no-deps --all-targets -- -D warnings
	cargo clippy --no-deps --manifest-path examples/raft-kv-memstore/Cargo.toml --all-targets -- -D warnings
	cargo clippy --no-deps --manifest-path examples/raft-kv-rocksdb/Cargo.toml  --all-targets -- -D warnings
	# Bug: clippy --all-targets reports false warning about unused dep in
	# `[dev-dependencies]`:
	# https://github.com/rust-lang/rust/issues/72686#issuecomment-635539688
	# Thus we only check unused deps for lib
	RUSTFLAGS=-Wunused-crate-dependencies cargo clippy --no-deps  --lib -- -D warnings

unused_dep:
	cargo machete

typos:
	# cargo install typos-cli
	typos --write-changes openraft/ tests/ memstore/ rocksstore rocksstore-compat07/ examples/raft-kv-memstore/ examples/raft-kv-rocksdb/
	# typos

clean:
	cargo clean

.PHONY: test fmt lint clean doc guide
