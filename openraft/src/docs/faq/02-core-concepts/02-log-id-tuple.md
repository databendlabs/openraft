### Why is log id a tuple of `(term, node_id, log_index)`?

In standard Raft log id is `(term, log_index)`, in Openraft he log id `(term,
node_id, log_index)` is used to minimize the chance of election conflicts.
This way in every term there could be more than one leader elected, and the last one is valid.
See: [`leader-id`](`crate::docs::data::leader_id`) for details.

[`leader-id`]: `crate::docs::data::leader_id`
