# Leader-id in Advanced mode and Standard mode

Openraft provides two `LeaderId` types to switch between these two modes with feature `single-term-leader`:

### Default: advanced mode

`cargo build` without [`single-term-leader`], is the advanced mode, the default mode:
`LeaderId` is defined as the following, and it is a **totally ordered** value(two or more leaders can be granted in the same term):

```ignore
// Advanced mode(default):
#[cfg(not(feature = "single-term-leader"))]
#[derive(PartialOrd, Ord)]
pub struct LeaderId<NID: NodeId>
{
  pub term: u64,
  pub node_id: NID,
}
```

Which means, in a single `term`, there could be more than one leaders
elected(although only the last is valid and can commit logs).

- Pros: election conflict is minimized,

- Cons: `LogId` becomes larger: every log has to store an additional `NodeId` in `LogId`:
  `LogId: {{term, NodeId}, index}`.
  If an application uses a big `NodeId` type, e.g., UUID, the penalty may not
  be negligible.


#### Standard mode

`cargo build --features "single-term-leader"` builds openraft in standard raft mode.
In the standard mode, `LeaderId` is defined as the following, and it is a **partially ordered** value(no two leaders can be granted in the same term):

```ignore
// Standard raft mode:
#[cfg(feature = "single-term-leader")]
pub struct LeaderId<NID: NodeId>
{
  pub term: u64,
  pub voted_for: Option<NID>,
}

impl<NID: NodeId> PartialOrd for LeaderId<NID> {

  fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
    match PartialOrd::partial_cmp(&self.term, &other.term) {
    
      Some(Ordering::Equal) => {
        match (&self.voted_for, &other.voted_for) {
          (None, None) => Some(Ordering::Equal),
          (Some(_), None) => Some(Ordering::Greater),
          (None, Some(_)) => Some(Ordering::Less),
          (Some(a), Some(b)) => { if a == b { Some(Ordering::Equal) } else { None } }
        }
      }
      cmp => cmp,
    }
  }
}
```

In this mode, only one leader can be elected in each `term`.

The partial order relation of `LeaderId`:

```ignore
LeaderId(3, None)    >  LeaderId(2, None):    true
LeaderId(3, None)    >  LeaderId(2, Some(y)): true
LeaderId(3, None)    == LeaderId(3, None):    true
LeaderId(3, Some(x)) >  LeaderId(2, Some(y)): true
LeaderId(3, Some(x)) >  LeaderId(3, None):    true
LeaderId(3, Some(x)) == LeaderId(3, Some(x)): true
LeaderId(3, Some(x)) >  LeaderId(3, Some(y)): false
```

The partial order between `Vote` is defined as:
Given two `Vote` `a` and `b`:
`a > b` iff:

```ignore
a.leader_id > b.leader_id || (
  !(a.leader_id < b.leader_id) && a.committed > b.committed
)
```

In other words, if `a.leader_id` and `b.leader_id` is not
comparable(`!(a.leader_id>=b.leader_id) && !(a.leader_id<=b.leader_id)`), use
field `committed` to determine the order between `a` and `b`.

Because a leader must be granted by a quorum before committing any log, two
incomparable `leader_id` can not both be granted.
So let a committed `Vote` override a incomparable non-committed is safe.

- Pros: `LogId` just store a `term`.

- Cons: election conflicting rate may increase.


