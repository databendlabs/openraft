# FAQ

-   **🤔 Why is log id a tuple of `(term, node_id, log_index)`, while standard Raft uses just
    `(term, log_index)`**?

    💡 The log id `(term, node_id, log_index)` is used to minimize the chance of election conflicts.
    This way in every term there could be more than one leaders elected, and the last one is valid.
    See: [`leader-id`](`crate::docs::data::leader_id`) for details.
    <br/><br/>


-   **🤔 How to remove node-2 safely from a cluster `{1, 2, 3}`**?

    💡 Call `Raft::change_membership(btreeset!{1, 3})` to exclude node-2 from
    the cluster. Then wipe out node-2 data.
    **NEVER** modify/erase the data of any node that is still in a raft cluster, unless you know what you are doing.
    <br/><br/>


-   **🤔 Can I wipe out the data of **one** node and wait for the leader to replicate all data to it again**?

    💡 Avoid doing this. Doing so will panic the leader. But it is permitted
    if [`loosen-follower-log-revert`] feature flag is enabled.

    In a raft cluster, although logs are replicated to multiple nodes,
    wiping out a node and restarting it is still possible to cause data loss.
    Assumes the leader is `N1`, followers are `N2, N3, N4, N5`:
    - A log(`a`) that is replicated by `N1` to `N2, N3` is considered committed.
    - At this point, if `N3` is replaced with an empty node, and at once the leader `N1` is crashed. Then `N5` may elected as a new  leader with granted vote by `N3, N4`;
    - Then the new leader `N5` will not have log `a`.

    ```text
    Ni: Node i
    Lj: Leader   at term j
    Fj: Follower at term j

    N1 | L1  a  crashed
    N2 | F1  a
    N3 | F1  a  erased          F2
    N4 |                        F2
    N5 |                 elect  L2
    ----------------------------+---------------> time
                                Data loss: N5 does not have log `a`
    ```

    But for even number nodes cluster, Erasing **exactly one** node won't cause data loss.
    Thus, in a special scenario like this, or for testing purpose, you can use
    `--feature loosen-follower-log-revert` to permit erasing a node.
    <br/><br/>


[`loosen-follower-log-revert`]: `crate::docs::feature_flags#loosen_follower_log_revert`
