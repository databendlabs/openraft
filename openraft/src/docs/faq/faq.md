# FAQ

-   Q: 🤔 Why is log id `(term, node_id, log_index)`, while standard Raft uses just
    `(term, log_index)`?

    A: The log id `(term, node_id, log_index)` is used to minimize the chance of election conflicts.
    This way in every term there could be more than one leaders elected, and the last one is valid.
    See: [`leader-id`](`crate::docs::data::leader_id`) for details.
