#!/bin/bash

set -o errexit

cargo fmt
cargo test --lib
cargo test --test '*'
cargo clippy --no-deps --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --document-private-items --all --no-deps
