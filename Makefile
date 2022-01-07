all: test lint fmt defensive_test

defensive_test:
	RAFT_STORE_DEFENSIVE=on cargo test

test: lint fmt
	cargo test

fmt:
	cargo fmt

lint:
	cargo fmt
	cargo clippy --all-targets -- -D warnings -A clippy::bool-assert-comparison

clean:
	cargo clean

.PHONY: test fmt lint clean
