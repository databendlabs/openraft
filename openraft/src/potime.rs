/// Partially ordered time that is used in distributed system.
///
/// See: [poset](https://en.wikipedia.org/wiki/Partially_ordered_set).
/// Consensus algorithm such as paxos and raft has their own **time** defined, which is why they do
/// not reply on wall-clock to achieve correctness.
///
/// In paxos the **time** is ballot-number, which is a totally ordered value.
/// In standard raft it is `(term:u64, voted_for:Option<NodeId>)`, which is a partially ordered
/// value.
pub trait PoTime: PartialOrd + PartialEq + Eq {}
