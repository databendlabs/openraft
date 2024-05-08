#!/bin/sh

# Appearence order is disabled
# RUSTDOCFLAGS='-Z unstable-options --sort-modules-by-appearance' cargo watch -x 'doc --document-private-items --all --no-deps'

if [ ".$1" = ".-p" ]; then
    cargo watch -x 'doc --document-private-items --all --no-deps'
else
    cargo watch -x 'doc --all --no-deps'
fi
