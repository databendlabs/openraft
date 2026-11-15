# Relaxed read benchmark results

These are the results of every grid of `scripts/relaxed_reads.py`, run on 2026-11-15. They
measure what `LinearizerOption::with_relaxed()` gains, and what the follower apply bound of
`Config::enable_relaxed_read` costs. How to run each grid is described in the
[README](README.md#comparing-core-local-and-relaxed-reads).

## Setup

- Apple M3 Max (16 cores), macOS 14.5. Desktop applications kept the load average between about 7
  and 13 during the runs.
- Openraft at `129fce97` on branch `relaxed-leader-only-reads`. The leader-change grid ran at
  `4d216f7e`, which only fixes that grid's operation counts.
- `bench` built in release mode without runtime statistics: three voters in one process,
  in-memory storage, no network delay, one client worker thread and 16 server threads, 128 warmup
  writes.
- Raft config: ticks, heartbeats and elections disabled, no snapshots. Each mode sets
  `enable_local_lease_read` and `enable_relaxed_read` as its name says.
- Every read includes its apply barrier and the same query: a write counter and its applied log
  ID, sampled under one read lock of the state machine.
- Mixed workloads schedule 90,000 reads/s from 64 readers and 10,000 writes/s from 32 writers for
  two seconds. Latency is measured from each operation's scheduled arrival, so it includes any
  backlog. Throughput uses total elapsed time.
- Each cell is the median of three runs, with the order of the modes rotated between runs.

Reproduce one grid with:

```sh
python3 benchmarks/minimal/scripts/relaxed_reads.py --grid <grid> --runs 3 --raw /tmp/<grid>.jsonl
```

## Leader reads (`--grid leader`)

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

- Without an apply backlog, relaxed reads gain nothing: read-only runs match the cached local path
  (about 6.5 M reads/s, 0.125 µs p50), and both are far ahead of reads through RaftCore.
- With a 500 µs apply-log-read delay, relaxed reads have the lowest latency: p50 956 µs against
  1,212 µs (local) and 1,317 µs (core), and p99 2.15 ms against 2.87 ms and 3.53 ms.
- With a 500 µs state-machine I/O delay the gain is smaller (p50 1,106 µs against 1,204 µs),
  because the store holds its write lock during the I/O and every query waits for that lock.

## Apply-delay sweep (`--grid sweep`)

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

- At 100 µs the three modes are within noise of each other.
- At a 2 ms apply-log-read delay the apply path cannot keep up with 10,000 writes/s (about 5,300
  writes/s complete), the committed backlog grows during the run, and reads that wait for the
  committed log ID take seconds: p50 2.34 s (core) and 2.32 s (local). Relaxed reads skip that
  backlog and stay at 1.2 ms p50.
- At a 2 ms state-machine I/O delay every mode takes seconds, relaxed reads too (p50 2.08 s),
  because the query waits for the store's write lock rather than for the apply barrier.

## Follower reads under the apply bound (`--grid follower`)

Standard reads on follower 1: the leader issues a cached LeaseRead linearizer, and the follower
waits for it and runs the query. `follower+bound` only enables `enable_relaxed_read`. The lag
columns are the follower's p99 apply lag behind the leader.

| Workload | Mode | Reads/s | Read p50 µs | Read p99 µs | Writes/s | Lag p99 entries | Lag p99 µs |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| mixed/no-delay | follower | 89,837 | 1174.598 | 2283.069 | 9,982 | 0 | 70 |
| mixed/no-delay | follower+bound | 89,841 | 1173.722 | 2239.667 | 9,982 | 0 | 96 |
| mixed/log-read-500us | follower | 89,827 | 1804.792 | 3227.791 | 9,981 | 0 | 153 |
| mixed/log-read-500us | follower+bound | 89,277 | 95021.763 | 336213.542 | 9,920 | 10 | 943 |
| mixed/apply-io-500us | follower | 89,824 | 1958.056 | 4284.556 | 9,980 | 0 | 220 |
| mixed/apply-io-500us | follower+bound | 88,519 | 335769.875 | 765968.195 | 9,835 | 10 | 857 |

- The bound keeps followers behind the leader: with an apply delay, the median time lag rises from
  under 30 µs to about 710 µs, and the p99 lag to 10 entries.
- With an apply delay, follower reads under the bound are far slower: p50 95 ms instead of 1.8 ms
  with the apply-log-read delay, and 336 ms instead of 2.0 ms with the I/O delay. All three runs
  agree. Without delays there is no difference.
- That slowdown is not explained yet. The apply lag alone is under 1.1 ms at p99, and the
  slowdown stays with more client threads: in single runs with 4 and 8 client workers, p50 was
  207 ms and 246 ms with the bound, against 1.7 ms and 2.0 ms without it. It needs investigating
  before it is taken as the cost of the bound.
- The lag is measured from when each node publishes its metrics, as seen by a task on the busy
  client runtime. Differences of a few tens of entries or about 100 µs are within that error;
  this is also why the bound shows occasional negative samples.

## Write cost of the apply bound (`--grid writes`)

Unpaced write-only runs. With the setting, every apply result on the leader sends the bound to the
replication streams again.

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

- No cost is measurable: the two modes differ by at most about 9%, in both directions, which is
  within the run-to-run noise.

## Replication change apart from the read barrier (`--grid bound`)

`core+bound` and `local+bound` enable the setting without requesting relaxed reads.

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

- The replication change alone (`core` against `core+bound`) costs nothing without delays, about
  4% p50 with the apply-log-read delay, and 15% p50 (p99 5.6 ms to 7.5 ms) with the I/O delay.
- The relaxed barrier alone (`local+bound` against `relaxed`) cuts p50 from 1,324 µs to 946 µs
  with the apply-log-read delay, and from 2,241 µs to 1,432 µs with the I/O delay.

## Relaxed and standard ReadIndex (`--grid read-index`)

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

- Read-only runs show no difference: the quorum round trip dominates.
- With an apply delay, relaxed ReadIndex reads are faster: p50 1,026 µs against 1,473 µs with the
  apply-log-read delay, and p99 6.1 ms against 17.8 ms with the I/O delay.

## Reads across a leader change (`--grid leader-change`)

Leadership moves from node 0 to node 1 one second into a three-second mixed workload. "Election"
is from the transfer request until node 1 leads, and "First read" from then until the first read
on node 1 finished. The first-second columns cover reads due in the first second after the
election; the last column covers the whole run.

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
  the apply-log-read delay, and 1.5 ms against 2.4 ms with the I/O delay.

## What these numbers do not show

- Effects that need more client threads. All readers and writers share one client worker thread,
  which also limits how fast closed-loop readers can follow the 90,000 reads/s schedule.
- Real storage and networks. The store and network are in memory; the delays are fixed per apply
  batch, and there is no network delay.
- Stores that do not lock queries out during apply. This store holds its write lock during the
  simulated I/O, so I/O-delay runs measure lock waits as much as apply barriers.
- Stable absolute numbers. Each run lasts two or three seconds on a busy desktop, and the same
  workload and mode differ between grids: `mixed/apply-io-500us` in the `local` mode has p50
  1,204 µs in the leader grid, 2,053 µs in the bound grid and 2,111 µs in the sweep grid.
  Compare modes within one grid, where the runs interleave, rather than across grids.
