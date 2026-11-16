# Relaxed read benchmark results

These are the results of every grid of `scripts/relaxed_reads.py`, run on 2026-11-15, through
the sequential replication path (now `--sequential-append`), the only one the bench had then. A
grid is a set of workloads, each run in several modes. The grids measure what relaxed reads gain,
and what the follower apply bound costs. Both terms are defined below. How to run a grid is
described in the [README](README.md#comparing-core-local-and-relaxed-reads).

## What is compared

### Read barriers

A linearizable read first confirms that the leader still leads: through RaftCore, or through a
lease cached outside RaftCore. It then waits until a node's state machine has applied a log ID,
the read barrier, and only then queries the state machine.

- A **standard read** waits for `max(no-op, committed)`: the no-op log of the current leadership,
  and the log ID that was committed when the read was authorized. If the leader commits faster
  than it applies, the read waits for that whole apply backlog.
- A **relaxed read**, requested with `LinearizerOption::with_relaxed(true)`, waits only for the
  no-op of the current leadership. It skips the apply backlog. Writes that already responded are
  still visible, because Openraft requires a write to be visible in the state machine before it
  responds.

### The follower apply bound

`Config::enable_relaxed_read` does two things:

1. The leader grants relaxed reads to requests that ask for them.
2. The leader limits how far followers may apply. Normally it sends followers its committed log ID
   as `leader_commit`, and they apply up to it, sometimes before the leader has applied the same
   entries itself. With the setting it sends `min(committed, leader's applied log ID)` instead,
   and sends it again after each of its own applies. No follower then applies an entry before the
   leader does, which relaxed reads need for safety.

This report calls the second part **the bound**. It is in effect as soon as the setting is on,
whether or not any read is relaxed. Its cost is that followers apply later, so standard reads
served on followers wait longer. A mode named `+bound` turns the setting on without requesting
relaxed reads, so that the bound is its only difference from the mode without it.

### Modes

| Mode | `enable_local_lease_read` | `enable_relaxed_read` | How each read runs |
| --- | --- | --- | --- |
| `core` | false | false | RaftCore checks the lease; standard barrier on the leader |
| `local` | true | false | Cached lease check outside RaftCore; standard barrier on the leader |
| `relaxed` | true | true | Cached lease check; relaxed barrier on the leader |
| `core+bound` | false | true | As `core`, with the bound on |
| `local+bound` | true | true | As `local`, with the bound on |
| `follower` | true | false | The leader authorizes a standard read with its cached lease; follower 1 waits for the barrier and runs the query |
| `follower+bound` | true | true | As `follower`, with the bound on |
| `read-index` | false | false | RaftCore confirms leadership with a quorum (ReadIndex); standard barrier on the leader |
| `read-index+relaxed` | false | true | ReadIndex confirmation; relaxed barrier on the leader |
| `bound-off`, `bound-on` | false | false, true | No reads: write-only runs, with the bound off or on |

## Workloads

- `read-only/N`: N readers and no writers. Reads are unpaced: each reader starts its next read
  when the previous one finishes, and latency is measured from the start of each read. With no
  writes there is no apply backlog.
- `mixed/...`: 64 readers paced at 90,000 reads/s together and 32 writers paced at 10,000 writes/s
  together, for two seconds (three in the leader-change grid). Latency is measured from each
  operation's scheduled arrival time, so it includes the time an operation waits because its
  client is still busy with earlier ones.
- `write-only/N`: N writers and no readers, unpaced; latency is measured from the start of each
  write.

The suffix names an apply delay, which builds up an apply backlog on every node:

- `log-read-<T>us`: each apply batch waits T µs before its log entries are read
  (`--apply-read-delay-us`). Apply falls behind commit, but queries are not blocked.
- `apply-io-<T>us`: each `apply()` call waits T µs after applying its entries, while it holds the
  store's write lock (`--apply-io-delay-us`). Apply falls behind commit, the batch's write
  responses wait for the delay, and every query on that node waits for the lock.
- `no-delay`: neither delay.

## Setup

- Apple M3 Max (16 cores), macOS 14.5. Desktop applications kept the load average between about 7
  and 13 during the runs.
- Openraft and the benchmark as of the commit that adds this report. The runs used an earlier
  revision of this branch. Its code differs only in ways that do not change what is measured:
  documentation, `LinearizeState`'s `Display` output, one RaftCore read of
  `enable_relaxed_read` that now goes through `Config` and returns the same value, and two
  `#[since]` attributes since removed from the benchmark store. The leader-change grid ran
  after a fix to its operation counts, which no other grid uses.
- `bench` built in release mode without runtime statistics: three voters in one process, an
  in-memory log and state machine, an in-process network without delay, one client worker thread
  for all readers and writers, 16 server threads, and 128 warmup writes before each run.
- Raft config: ticks, heartbeats and elections disabled, no snapshots; the two read settings as in
  the mode table.
- Every read runs the same query after its barrier: a write counter and its applied log ID, read
  under one read lock of the state machine.
- Each cell is the median of three runs; the order of the modes rotates between runs. Throughput
  divides the completed operations by the run's total elapsed time.

Reproduce one grid with:

```sh
python3 benchmarks/minimal/scripts/relaxed_reads.py --grid <grid> --runs 3 --raw /tmp/<grid>.jsonl
```

## Leader reads (`--grid leader`)

**What it runs:** every workload with reads on the leader, in the `core`, `local` and `relaxed`
modes.
**Question:** how much faster relaxed reads are than standard reads, with and without an apply
backlog.

| Workload | Mode | Reads/s | Read p50 µs | Read p99 µs | Writes/s |
| --- | --- | ---: | ---: | ---: | ---: |
| read-only/1 | core | 94,103 | 9.500 | 23.208 | 0 |
| read-only/1 | local | 6,346,521 | 0.125 | 0.167 | 0 |
| read-only/1 | relaxed | 6,483,596 | 0.125 | 0.167 | 0 |
| read-only/64 | core | 1,785,348 | 29.875 | 76.917 | 0 |
| read-only/64 | local | 6,519,755 | 0.125 | 0.167 | 0 |
| read-only/64 | relaxed | 6,459,444 | 0.125 | 0.167 | 0 |
| mixed/no-delay | core | 89,848 | 1179.999 | 2229.625 | 9,983 |
| mixed/no-delay | local | 89,798 | 1138.097 | 2213.028 | 9,978 |
| mixed/no-delay | relaxed | 89,809 | 1134.472 | 2148.917 | 9,979 |
| mixed/log-read-500us | core | 89,806 | 1316.764 | 3531.139 | 9,978 |
| mixed/log-read-500us | local | 89,781 | 1212.069 | 2870.181 | 9,976 |
| mixed/log-read-500us | relaxed | 89,805 | 955.889 | 2154.028 | 9,978 |
| mixed/apply-io-500us | core | 89,754 | 1371.694 | 2861.722 | 9,973 |
| mixed/apply-io-500us | local | 89,778 | 1203.694 | 4250.556 | 9,975 |
| mixed/apply-io-500us | relaxed | 89,801 | 1106.222 | 2800.777 | 9,978 |

- Without an apply backlog relaxed reads gain nothing. In the read-only runs `relaxed` matches
  `local`, about 6.5 M reads/s at 0.125 µs p50; both avoid RaftCore, which `core` pays for.
- With a 500 µs log-read delay, relaxed reads have the lowest latency: p50 956 µs against
  1,212 µs (`local`) and 1,317 µs (`core`), and p99 2.15 ms against 2.87 ms and 3.53 ms.
- With a 500 µs I/O delay the gain is smaller (p50 1,106 µs against 1,204 µs): a relaxed read
  skips the apply wait but still waits for the store's write lock before its query.

## Apply-delay sweep (`--grid sweep`)

**What it runs:** the paced mixed workload with each delay at 0, 100 µs, 500 µs and 2 ms, in the
`core`, `local` and `relaxed` modes.
**Question:** how large the apply backlog must be before relaxed reads pay off.

| Workload | Mode | Reads/s | Read p50 µs | Read p99 µs | Writes/s |
| --- | --- | ---: | ---: | ---: | ---: |
| mixed/no-delay | core | 89,795 | 1315.458 | 2700.902 | 9,977 |
| mixed/no-delay | local | 89,784 | 1251.848 | 2505.362 | 9,976 |
| mixed/no-delay | relaxed | 89,801 | 1250.598 | 2435.902 | 9,978 |
| mixed/log-read-100us | core | 89,803 | 1156.569 | 2319.125 | 9,978 |
| mixed/log-read-100us | local | 89,801 | 1210.139 | 2493.792 | 9,978 |
| mixed/log-read-100us | relaxed | 89,831 | 1124.513 | 2343.292 | 9,981 |
| mixed/log-read-500us | core | 89,773 | 1452.306 | 4895.931 | 9,975 |
| mixed/log-read-500us | local | 89,827 | 2008.277 | 6040.777 | 9,981 |
| mixed/log-read-500us | relaxed | 89,784 | 924.652 | 2061.444 | 9,976 |
| mixed/log-read-2000us | core | 47,799 | 2335955.083 | 3137558.500 | 5,311 |
| mixed/log-read-2000us | local | 48,679 | 2320177.250 | 3155128.972 | 5,409 |
| mixed/log-read-2000us | relaxed | 48,285 | 1213.375 | 3324.611 | 5,365 |
| mixed/apply-io-100us | core | 89,829 | 1188.013 | 2390.208 | 9,981 |
| mixed/apply-io-100us | local | 89,779 | 1206.111 | 2592.028 | 9,975 |
| mixed/apply-io-100us | relaxed | 89,832 | 1198.139 | 2447.361 | 9,981 |
| mixed/apply-io-500us | core | 89,801 | 1621.903 | 8648.041 | 9,978 |
| mixed/apply-io-500us | local | 89,770 | 2110.653 | 9540.972 | 9,974 |
| mixed/apply-io-500us | relaxed | 89,780 | 1389.153 | 5144.751 | 9,976 |
| mixed/apply-io-2000us | core | 46,413 | 2513822.361 | 3340848.653 | 5,157 |
| mixed/apply-io-2000us | local | 46,763 | 2550903.819 | 3419707.319 | 5,196 |
| mixed/apply-io-2000us | relaxed | 47,194 | 2075393.153 | 2827829.750 | 5,244 |

- At 100 µs the three modes are within noise of each other: the backlog is too small to matter.
- At a 2 ms log-read delay, apply cannot keep up with 10,000 writes/s. Only about 5,300 writes/s
  complete, and the committed but unapplied backlog grows during the run. Standard reads wait for
  that backlog and take seconds: p50 2.34 s (`core`) and 2.32 s (`local`). Relaxed reads skip it
  and stay at 1.2 ms p50.
- At a 2 ms I/O delay every mode takes seconds, relaxed reads too (p50 2.08 s): queries wait for
  the store's write lock, which apply holds most of the time.

## Follower reads (`--grid follower`)

**What it runs:** the three mixed workloads, with every read served on follower 1 in the
`follower` and `follower+bound` modes. As in an application's follower read, the leader
authorizes each read, here with its cached lease, and hands over its barrier; follower 1 then
waits until it has applied that standard barrier and runs the query.
**Question:** what the bound costs follower reads, and how far it holds followers behind the
leader.

Two extra columns describe follower 1's apply lag. Every time follower 1 reports a new applied
log index, the benchmark records how many entries the leader had applied beyond it (Lag
entries), and how long after the leader the follower reached that index (Lag µs). The columns
show the p99 of each over the run.

| Workload | Mode | Reads/s | Read p50 µs | Read p99 µs | Writes/s | Lag p99 entries | Lag p99 µs |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| mixed/no-delay | follower | 89,837 | 1174.598 | 2283.069 | 9,982 | 0 | 70 |
| mixed/no-delay | follower+bound | 89,841 | 1173.722 | 2239.667 | 9,982 | 0 | 96 |
| mixed/log-read-500us | follower | 89,827 | 1804.792 | 3227.791 | 9,981 | 0 | 153 |
| mixed/log-read-500us | follower+bound | 89,277 | 95021.763 | 336213.542 | 9,920 | 10 | 943 |
| mixed/apply-io-500us | follower | 89,824 | 1958.056 | 4284.556 | 9,980 | 0 | 220 |
| mixed/apply-io-500us | follower+bound | 88,519 | 335769.875 | 765968.195 | 9,835 | 10 | 857 |

- Without the bound, a follower applies an entry as soon as it learns that the entry is committed,
  usually within 30 µs of the leader (median). With the bound it must wait until the leader has
  applied the entry and sent the new bound: with an apply delay, the median lag rises to about
  710 µs, and the p99 lag to 10 entries.
- With an apply delay, follower reads under the bound are far slower: p50 95 ms instead of 1.8 ms
  with the log-read delay, and 336 ms instead of 2.0 ms with the I/O delay. All three runs agree.
  Without delays there is no difference.
- That slowdown is not explained yet. The apply lag is under 1.1 ms even at p99, and the
  slowdown stays with more client threads: in single runs with 4 and 8 client worker threads,
  p50 was 207 ms and 246 ms with the bound, against 1.7 ms and 2.0 ms without it. It needs
  investigating before it is taken as the cost of the bound.
- The lag compares when each node publishes its metrics, as seen by a task on the busy client
  runtime. Differences of a few tens of entries or about 100 µs are within that error, which is
  also why some samples show a follower ahead of the leader even with the bound on.

## Write cost of the bound (`--grid writes`)

**What it runs:** unpaced write-only workloads with 1, 32 and 256 writers and each delay, with
the bound off and on.
**Question:** what re-sending the bound after each of the leader's applies costs writes. Each
re-send wakes the replication streams, which send the new bound to the followers.

| Workload | Mode | Writes/s | Write p50 µs | Write p99 µs |
| --- | --- | ---: | ---: | ---: |
| write-only/1 | bound-off | 30,857 | 30.125 | 58.125 |
| write-only/1 | bound-on | 33,766 | 26.042 | 59.542 |
| write-only/1/log-read-500us | bound-off | 1,438 | 690.208 | 818.000 |
| write-only/1/log-read-500us | bound-on | 1,463 | 679.083 | 769.834 |
| write-only/1/apply-io-500us | bound-off | 1,432 | 689.708 | 915.875 |
| write-only/1/apply-io-500us | bound-on | 1,457 | 679.542 | 879.000 |
| write-only/32 | bound-off | 622,797 | 49.333 | 82.208 |
| write-only/32 | bound-on | 653,263 | 45.958 | 85.750 |
| write-only/32/log-read-500us | bound-off | 25,062 | 1300.750 | 1366.000 |
| write-only/32/log-read-500us | bound-on | 24,630 | 1304.208 | 1369.625 |
| write-only/32/apply-io-500us | bound-off | 25,116 | 1300.958 | 1577.417 |
| write-only/32/apply-io-500us | bound-on | 25,291 | 1304.792 | 1356.917 |
| write-only/256 | bound-off | 1,604,961 | 153.542 | 244.292 |
| write-only/256 | bound-on | 1,615,845 | 154.584 | 244.583 |
| write-only/256/log-read-500us | bound-off | 191,923 | 1336.208 | 1429.791 |
| write-only/256/log-read-500us | bound-on | 190,338 | 1338.500 | 1531.959 |
| write-only/256/apply-io-500us | bound-off | 194,695 | 1322.166 | 1403.292 |
| write-only/256/apply-io-500us | bound-on | 191,556 | 1337.541 | 1417.750 |

- No cost is measurable. The two modes differ by at most about 9%, in both directions, which is
  within the noise between runs.

## The bound apart from the relaxed barrier (`--grid bound`)

**What it runs:** the three mixed workloads in five modes. `core+bound` and `local+bound` turn the
bound on without requesting relaxed reads.
**Question:** how much of the `relaxed` mode's result comes from the relaxed barrier, and how much
from the bound. `relaxed` has both.

| Workload | Mode | Reads/s | Read p50 µs | Read p99 µs | Writes/s |
| --- | --- | ---: | ---: | ---: | ---: |
| mixed/no-delay | core | 89,800 | 1184.431 | 2191.251 | 9,978 |
| mixed/no-delay | core+bound | 89,798 | 1184.625 | 2298.681 | 9,978 |
| mixed/no-delay | local | 89,846 | 1144.402 | 2168.625 | 9,983 |
| mixed/no-delay | local+bound | 89,784 | 1154.361 | 2223.306 | 9,976 |
| mixed/no-delay | relaxed | 89,834 | 1141.459 | 2183.902 | 9,982 |
| mixed/log-read-500us | core | 89,810 | 1299.347 | 2461.639 | 9,979 |
| mixed/log-read-500us | core+bound | 89,777 | 1350.792 | 2548.125 | 9,975 |
| mixed/log-read-500us | local | 89,806 | 1242.472 | 2919.320 | 9,978 |
| mixed/log-read-500us | local+bound | 89,786 | 1324.139 | 3351.722 | 9,976 |
| mixed/log-read-500us | relaxed | 89,768 | 945.958 | 2147.819 | 9,974 |
| mixed/apply-io-500us | core | 89,837 | 1593.542 | 5647.194 | 9,982 |
| mixed/apply-io-500us | core+bound | 89,781 | 1837.542 | 7511.180 | 9,976 |
| mixed/apply-io-500us | local | 89,785 | 2053.402 | 10299.166 | 9,976 |
| mixed/apply-io-500us | local+bound | 89,807 | 2240.556 | 10449.681 | 9,979 |
| mixed/apply-io-500us | relaxed | 89,812 | 1431.652 | 6908.458 | 9,979 |

- The bound alone (`core` against `core+bound`) costs leader reads nothing without delays, about
  4% p50 with the log-read delay, and 15% p50 with the I/O delay (p99 5.6 ms to 7.5 ms).
- The relaxed barrier alone (`local+bound` against `relaxed`, both with the bound) cuts p50 from
  1,324 µs to 946 µs with the log-read delay, and from 2,241 µs to 1,432 µs with the I/O delay.

## ReadIndex reads (`--grid read-index`)

**What it runs:** every workload with ReadIndex reads on the leader, standard and relaxed. A
ReadIndex read always runs in RaftCore and waits for a quorum to confirm that the leader still
leads, before its barrier.
**Question:** whether skipping the apply wait still matters when every read pays that round trip.

| Workload | Mode | Reads/s | Read p50 µs | Read p99 µs | Writes/s |
| --- | --- | ---: | ---: | ---: | ---: |
| read-only/1 | read-index | 54,811 | 16.834 | 39.417 | 0 |
| read-only/1 | read-index+relaxed | 55,404 | 16.750 | 37.708 | 0 |
| read-only/64 | read-index | 1,158,248 | 48.875 | 104.583 | 0 |
| read-only/64 | read-index+relaxed | 1,132,126 | 50.250 | 102.417 | 0 |
| mixed/no-delay | read-index | 89,841 | 1337.625 | 2559.861 | 9,982 |
| mixed/no-delay | read-index+relaxed | 89,784 | 1338.570 | 2675.973 | 9,976 |
| mixed/log-read-500us | read-index | 89,736 | 1473.125 | 3576.611 | 9,971 |
| mixed/log-read-500us | read-index+relaxed | 89,806 | 1025.820 | 2491.501 | 9,978 |
| mixed/apply-io-500us | read-index | 89,838 | 1947.987 | 17766.611 | 9,982 |
| mixed/apply-io-500us | read-index+relaxed | 89,781 | 1712.569 | 6110.890 | 9,976 |

- Read-only runs show no difference: there is no backlog, and the quorum round trip dominates.
- With an apply delay, relaxed ReadIndex reads are faster: p50 1,026 µs against 1,473 µs with the
  log-read delay, and p99 6.1 ms against 17.8 ms with the I/O delay.

## Reads across a leader change (`--grid leader-change`)

**What it runs:** a three-second mixed workload with a 500 µs delay. One second in, the benchmark
asks node 0 to transfer leadership to node 1, repeating the request every 5 ms until node 1
leads. Writers and readers follow: a request that a node rejects because leadership is moving
waits until node 1 leads, or for 1 ms, and retries on the current leader. The modes are `local`,
`local+bound` and `relaxed`.
**Question:** how long reads stall around an election, and whether the bound makes the new
leader slower to serve reads. With the bound, followers have applied less when they become
leader, so a new leader has more entries to apply before its own no-op.

The columns are:

- Election µs: from the first transfer request until node 1 reports that it leads.
- First read µs: from then until the first read served by node 1 finished.
- First-second read p50 µs, p99 µs: successful reads scheduled in the first second after the
  election, on any node.
- Read p99 µs: all reads of the run.

| Workload | Mode | Election µs | First read µs | First-second read p50 µs | p99 µs | Read p99 µs |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| mixed/log-read-500us/3s | local | 118 | 1,459 | 2,044 | 4,521 | 5,328 |
| mixed/log-read-500us/3s | local+bound | 164 | 1,339 | 2,164 | 5,118 | 5,356 |
| mixed/log-read-500us/3s | relaxed | 132 | 1,433 | 927 | 2,516 | 2,285 |
| mixed/apply-io-500us/3s | local | 189 | 1,592 | 2,400 | 14,086 | 13,181 |
| mixed/apply-io-500us/3s | local+bound | 154 | 2,009 | 2,453 | 14,182 | 16,461 |
| mixed/apply-io-500us/3s | relaxed | 134 | 2,137 | 1,511 | 8,373 | 6,614 |

- The election takes 120–190 µs in every mode, and the first read on the new leader finishes
  1.3–2.1 ms after it.
- The bound adds no visible failover cost here: `local` and `local+bound` are within noise of each
  other.
- Relaxed reads stay faster after the election: first-second p50 927 µs against about 2.1 ms with
  the log-read delay, and 1.5 ms against 2.4 ms with the I/O delay.

## What these numbers do not show

- Effects that need more client threads. All readers and writers share one client worker thread,
  which also limits how closely closed-loop readers can follow the 90,000 reads/s schedule.
- Real storage and networks. The log, state machine and network are in memory; the delays are
  fixed per apply batch, and there is no network delay.
- Stores that do not block queries during apply. This store holds its write lock during the
  simulated I/O, so the I/O-delay runs measure lock waits as much as read barriers.
- Stable absolute numbers. Each run lasts two or three seconds on a busy desktop, and the same
  workload and mode differ between grids: `mixed/apply-io-500us` in the `local` mode has p50
  1,204 µs in the leader grid, 2,053 µs in the bound grid and 2,111 µs in the sweep grid. Compare
  modes within one grid, where their runs interleave, rather than across grids.
