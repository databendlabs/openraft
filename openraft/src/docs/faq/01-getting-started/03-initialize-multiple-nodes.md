### Can I call `initialize()` on multiple nodes?

Calling `initialize()` on multiple nodes with **identical configuration** is
acceptable and will not cause consistency issues — the Raft voting protocol
ensures that only one leader will be elected.

However, calling `initialize()` with **different configurations** on different
nodes may lead to a split-brain condition and must be avoided.
