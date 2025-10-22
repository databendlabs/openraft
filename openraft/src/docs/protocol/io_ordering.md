# IO Ordering in Raft: Why Vote and Log Operations Cannot Be Reordered

This document explains why vote I/O and append-log I/O operations must maintain strict ordering in Raft. Reordering these operations can lead to committed data loss.


## Notation

```text
Ni:   Node i
Vi:   RequestVote with term=i
Li:   Establish Leader with term=i
Ei-j: Log entry at term=i and index=j

N5 |          V5  L5       E5-1
N4 |          V5           E5-1
N3 |  V1                V5,E5-1  E1-1
N2 |  V1      V5                 E1-1
N1 |  V1  L1                     E1-1
------+---+---+---+--------+-----+-------------------------------------------> time
      t1  t2  t3  t4       t5    t6
```


## Timeline Analysis

- **t1**: N1 initiates an election with term=1, receiving votes from N1, N2, and N3
- **t2**: N1 establishes itself as leader L1
- **t3**: N5 initiates an election with term=5, receiving votes from N5, N4, and N2
- **t4**: N5 establishes itself as leader L5
- **t5**: L5 replicates its first log entry E5-1 to N4 and N3
  - **Critical point**: N3's stored term (1) is less than the term in the AppendEntries RPC (5)
  - N3 must persist term=5 first, then persist log entry E5-1
  - Unlike N4 and N5 (which only need one I/O for E5-1), N3 requires two sequential I/O operations:
    1. Persist term=5
    2. Persist E5-1
- **t6**: L1 attempts to replicate E1-1 (term=1, index=1)


## The Ordering Problem


### Safe Behavior (No Reordering)

When I/O operations **cannot be reordered**, N3 executes them sequentially:
1. First: persist term=5 (from V5)
2. Second: persist E5-1

This ensures that if E5-1 is persisted, term=5 is also guaranteed to be persisted.


### Unsafe Behavior (Reordering Allowed)

If I/O operations **can be reordered**, the following sequence becomes possible:
1. Persist E5-1
2. Persist term=5 (from V5)

**The danger**: If the server crashes and restarts after E5-1 is written but before term=5 is persisted:
- N3's stored term remains 1
- E5-1 exists in the log
- When N3 receives L1's replication request for E1-1 (term=1, index=1):
  - N3 accepts the request because its term (1) matches L1's term (1)
  - **E1-1 overwrites E5-1**

**Critical violation**: E5-1 was already replicated to 3 nodes (N5, N4, N3), so L5 considers it committed. Allowing I/O reordering can cause this committed entry to be lost.


## Conclusion

Vote and log I/O operations must be strictly ordered to prevent committed data
loss.
The term update must be persisted before any log entries from that term,
ensuring that once a log entry is written, the corresponding term is also
guaranteed to be durable.

## See Also

- [IOId](crate::docs::data::io_id) - I/O operation identifier that treats vote and log as a unified whole
- [Log I/O Progress](crate::docs::data::log_io_progress) - Three-stage I/O progress tracking
