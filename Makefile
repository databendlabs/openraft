all: test lint fmt defensive_test doc

defensive_test:
	RAFT_STORE_DEFENSIVE=on cargo test

test: lint fmt
	cargo test
	cargo test --manifest-path example-raft-kv/Cargo.toml

fmt:
	cargo fmt

doc:
	cargo doc --all --no-deps

lint:
	cargo fmt
	cargo fmt --manifest-path example-raft-kv/Cargo.toml
	cargo clippy --all-targets -- -D warnings -A clippy::bool-assert-comparison
	cargo clippy --manifest-path example-raft-kv/Cargo.toml --all-targets -- -D warnings -A clippy::bool-assert-comparison

clean:
	cargo clean

.PHONY: test fmt lint clean doc
