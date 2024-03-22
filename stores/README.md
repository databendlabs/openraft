Example Storage implementations.

- `memstore` is in-memory storage and is used by the test cases `./tests`.

If a crate has different feature flags enabled, it must not be members of the workspace.
A feature flag will be enabled for the entire workspace if a member crate enables it.
