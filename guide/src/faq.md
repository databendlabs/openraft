# FAQ

-   Q: 🤔 Why is heartbeat an append-entries RPC with a blank log in openraft, while standard Raft uses empty append-entries?

    Chapter [Heartbeat](./heartbeat.md) explains the benefit of the heartbeat-log design.

-   Q: 🤔 Why is log id `(term, node_id, log_index)`, while standard Raft uses just
    `(term, log_index)`?

    TODO
