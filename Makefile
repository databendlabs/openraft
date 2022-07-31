all: test lint fmt defensive_test send_delay_test doc

defensive_test:
	OPENRAFT_STORE_DEFENSIVE=on cargo test

send_delay_test:
	OPENRAFT_NETWORK_SEND_DELAY=30 cargo test

test: lint fmt
	cargo test
	cargo test --manifest-path examples/raft-kv-memstore/Cargo.toml
	cargo test --manifest-path examples/raft-kv-rocksdb/Cargo.toml

bench_cluster_of_1:
	cargo test --package openraft --test benchmark --release bench_cluster_of_1 -- --ignored --nocapture

bench_cluster_of_3:
	cargo test --package openraft --test benchmark --release bench_cluster_of_3 -- --ignored --nocapture

bench_cluster_of_5:
	cargo test --package openraft --test benchmark --release bench_cluster_of_5 -- --ignored --nocapture

fmt:
	cargo fmt

fix:
	cargo fix --allow-staged

doc:
	cargo doc --all --no-deps

lint:
	cargo fmt
	cargo fmt --manifest-path examples/raft-kv-memstore/Cargo.toml
	cargo clippy --all-targets -- -D warnings -A clippy::bool-assert-comparison
	cargo clippy --manifest-path examples/raft-kv-memstore/Cargo.toml --all-targets -- -D warnings -A clippy::bool-assert-comparison

clean:
	cargo clean

.PHONY: test fmt lint clean doc
