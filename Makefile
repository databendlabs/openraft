all: test lint

test:
	cargo test

fmt:
	cargo fmt

lint:
	cargo fmt
	cargo clippy --all-targets -- -D warnings

clean:
	cargo clean

.PHONY: test fmt lint clean
