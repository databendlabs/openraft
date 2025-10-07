#!/bin/bash

set -o errexit

cargo test --lib
cargo test --test '*'
cargo clippy --no-deps --all-targets -- -D warnings
