# Feature flags

By default openraft enables no features.

- `bt`: attaches backtrace to generated errors.

- `serde`: derives `serde::Serialize, serde::Deserialize` for type that are used
    in storage and network, such as `Vote` or `AppendEntriesRequest`.

- `single-term-leader`: allows only one leader to be elected in each `term`.
    This is the standard raft policy, which increases election confliction rate
    but reduce `LogId`(`(term, node_id, index)` to `(term, index)`) size.
    Read more about how it is implemented in [`vote`](./vote.md)
