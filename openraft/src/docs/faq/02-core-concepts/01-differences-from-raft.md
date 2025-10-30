### What are the differences between Openraft and standard Raft?

- Optionally, In one term there could be more than one leader to be established, in order to reduce election conflict. See: std mode and adv mode leader id: [`leader_id`][];
- Openraft stores committed log id: See: [`RaftLogStorage::save_committed()`][];
- Openraft optimized `ReadIndex`: no `blank log` check: [`Linearizable Read`][].
- A restarted Leader will stay in Leader state if possible;
- Does not support single step membership change. Only joint is supported.

[`Linearizable Read`]: `crate::docs::protocol::read`
[`leader_id`]: `crate::docs::data::leader_id`
[`RaftLogStorage::save_committed()`]: `crate::storage::RaftLogStorage::save_committed`
