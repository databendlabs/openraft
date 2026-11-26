# Catch-up of a cut-off follower on main

## Summary

- Apply did not hold back a follower's catch-up when the state machine's cost is per `apply()`
  call. A follower cut off for 1 s fell 10,000 to 249,000 entries behind. Once its log reached
  the leader's commit index at the reconnection, it applied the rest within 0 to 120 ms. With
  10 µs per entry, applying took 540 to 790 ms more, about 10.4 ms per 1,024 entries missed.
- Receiving the log took longer, and the replication backoff set that time. With the default
  backoff, the follower got its first new entries 201 to 203 ms after the reconnection. Its log
  reached the target 605 to 640 ms after the reconnection, whether it was 10,000 or 249,000
  entries behind: three intervals of `Config::backoff`, 200 ms by default. With `backoff` set to
  10 ms, the log reached the target in 34 to 68 ms.
- The follower applied exactly 1,024 entries per `apply()` call while catching up, in every row:
  the state-machine worker reads at most 1,024 entries ahead of the call that is running. With
  2 ms calls that made 52 calls for 52,000 entries, 120 ms.
- The leader kept committing at the same rate. In the paced rows, writes/s stayed within 1% of
  the rate and client p99 within 0.3 ms, in the outage, during catch-up and after it. In some
  saturated runs the outage moved the 256 clients between the two patterns of writes per call
  that [the apply-lag report](../2026-11-25-apply-lag/report-full.md) describes, and the run kept
  the new pattern; the catch-up itself did not change the leader's rate.
- On the follower, committed but unapplied entries peaked at 4,000 to 180,000, up to 1.6 times
  the entries it missed. A returning follower's apply backlog grows with how far behind it is,
  not with the writes in flight.

## What was measured

The target is `benchmarks/minimal` at `5ded220b5cebc1aef353845dfef28056490159e2` (`ups/main`)
with this change: three voters in one process, an in-memory log store, empty `client_write()`
calls, and a network of in-process function calls. Every delay is a sleep on a blocking-pool
thread. The state machines and delay profiles are those of the apply-lag report:

| profile | append | flush | network, one way | apply read | apply I/O per call | per entry |
| :--     | --:    | --:   | --:              | --:        | --:                | --:       |
| `lan`   | 20     | 200   | 100              | 50         | 100, 500 or 2,000  | 0, or 10 with 100 µs per call |
| `cloud` | 50     | 1,000 | 500              | 100        | 500                | 0         |

In µs. Each run has 256 clients, each with one write in flight. 1 s after the clients start,
`--cut-follower-after-ms 1000 --cut-follower-ms 1000` cuts node 2 off from nodes 0 and 1 for 1 s,
or 5 s in two rows, then reconnects it. Every RPC from or to node 2 fails as `Unreachable`, and
an open append stream ends at once, losing what is on the wire. Node 0 stays leader with node 1.
A paced run writes at 10,000 writes/s and a saturated run unpaced, for about 3 s after the
reconnection.

The runs set `snapshot_policy` to `Never`, so the follower catches up from the log and applies
every entry it missed; a catch-up through a snapshot is not measured. The bench turns on
`enable_pre_vote` for these runs. Without it, the cut-off follower raises its term at each
election timeout, and on its return its higher vote makes the leader step down.

For each run the bench reports, from node 2's metrics after the reconnection:

- **entries behind**: the leader's committed index at the reconnection, the *target*, minus the
  follower's applied index then.
- **first new entry**, **log reaches target**, **applied target**: when the follower's last log
  index first passed its value at the reconnection, when it reached the target, and when its
  applied index did. **apply after log** is the difference of the last two.
- **follower max committed unapplied**: the follower's committed index minus its applied index,
  at its largest in its metrics until it applied the target.
- **apply() calls**: the follower's `apply()` calls from the reconnection until it applied the
  target, and their entries.
- **leader writes per phase**: writes/s and client p99 of the writes that completed before the
  cut, during the outage, during catch-up (from the reconnection until the follower applied the
  target), and after it.

## Method

- Machine: Apple M3 Max (16 cores), macOS 14.5, rustc 1.96.0-nightly (2026-02-28). One client
  worker thread, 16 server worker threads, 1,024 warmup writes, the bench's default `Config`
  with `snapshot_policy` `Never` and `enable_pre_vote`, release build with the default
  `runtime-stats` feature.
- Each row ran 5 times. Every round ran all rows once, in an order shuffled with seed 1812.
  Tables show medians. The 1-minute load average at the start of a run ranged from 4.2 to 10.7
  (median 6.0).
- All 70 runs caught up, and no write failed.

## Results

Complete tables, with a run-by-run table of the saturated rows, are in `results/tables.md`.

### Follower catch-up

Times in ms after the reconnection.

| state machine                   | load      | outage | entries behind | first new entry | log reaches target | applied target | apply after log | follower max committed unapplied | `apply()` calls |
| :--                             | --:       | :--    | --:            | --:             | --:                | --:            | --:             | --:                              | --:             |
| lan, apply 100 µs/call          | 10,000/s  | 1 s    |         10,010 |             201 |                605 |            605 |             0.0 |                            4,846 |              11 |
| lan, apply 100 µs/call          | saturated | 1 s    |        248,624 |             202 |                640 |            649 |             8.7 |                           64,512 |             261 |
| lan, apply 500 µs/call          | 10,000/s  | 1 s    |         10,010 |             202 |                605 |            610 |             4.2 |                           12,988 |              10 |
| lan, apply 500 µs/call          | saturated | 1 s    |        142,279 |             201 |                627 |            690 |            65.9 |                          180,388 |             140 |
| lan, apply 2 ms/call            | 10,000/s  | 1 s    |         10,010 |             203 |                605 |            625 |            19.5 |                           15,046 |              10 |
| lan, apply 2 ms/call            | saturated | 1 s    |         52,463 |             202 |                614 |            734 |           119.8 |                           78,974 |              52 |
| lan, 100 µs/call + 10 µs/entry  | 10,000/s  | 1 s    |         10,010 |             201 |                606 |            709 |           101.9 |                           15,136 |              10 |
| lan, 100 µs/call + 10 µs/entry  | saturated | 1 s    |         72,810 |             202 |                618 |          1,409 |           791.0 |                          116,842 |              72 |
| cloud, apply 500 µs/call        | 10,000/s  | 1 s    |         10,010 |             202 |                608 |            611 |             3.6 |                           11,994 |              11 |
| cloud, apply 500 µs/call        | saturated | 1 s    |         67,962 |             203 |                620 |            650 |            25.5 |                           76,768 |              68 |
| lan, apply 2 ms/call            | 10,000/s  | 5 s    |         50,010 |             201 |                611 |            717 |           107.1 |                           53,058 |              49 |
| lan, 100 µs/call + 10 µs/entry  | 10,000/s  | 5 s    |         50,010 |             202 |                610 |          1,153 |           542.1 |                           55,146 |              49 |
| lan, apply 100 µs/call, backoff 10 ms | 10,000/s  | 1 s | 10,010 |              12 |                 34 |             35 |             0.6 |                            4,206 |              10 |
| lan, apply 100 µs/call, backoff 10 ms | saturated | 1 s | 236,560 |             12 |                 68 |             73 |             5.1 |                           45,629 |             232 |

Every `apply()` call of every row applied 1,024 entries, at p50 and at most.

**The log.** The first new entries came one backoff interval after the reconnection, and the
log reached the target two intervals after that, in every row: 605 ms with 10,000 entries
behind, 640 ms with 249,000. Moving the entries took a few tens of milliseconds; the rows with
`backoff` set to 10 ms show it, at 34 and 68 ms. The replication task counts each failed RPC
toward a backoff (`Unreachable` weighs 100, and above 20 the task waits `Config::backoff`
before each request), so after an outage its next requests wait. I did not trace why it waits
twice more after the follower answered the first request.

**Apply, with a cost per call.** Once the log reached the target, the follower applied the rest
in 0 to 120 ms: about one call per 1,024 entries behind its log. With 2 ms calls, 52,463 entries
took 52 calls and 120 ms after the log; with 100 µs calls, 248,624 entries took 261 calls and
9 ms after the log, because most calls ran while the log was still arriving. The state-machine
worker admits apply commands while the entries admitted but not yet started fit a budget of
1,024 (`MAX_PREFETCH_ENTRIES` in `openraft/src/core/sm/worker.rs`), and a call takes only
admitted entries. So a call never took more than 1,024 entries, even with 180,000 committed
entries waiting. A larger budget would make fewer calls for a state machine
whose cost is per call, and hold more entries in memory; that was not measured.

**Apply, with a cost per entry.** With 10 µs per entry, applying set the catch-up time:
791 ms after the log for 72,810 entries, and 542 ms for 50,010. A call of 1,024 entries costs
10.24 ms, plus 100 µs per call and the 50 µs read, so 72 calls take 750 ms and 49 calls 510 ms.
The follower thus applied nearly all the entries it missed after its log reached the target,
at about 98,000 entries/s.

### Leader writes per phase

writes/s and client p99 in ms.

| state machine                   | load      | outage | before        | outage        | catch-up      | after         |
| :--                             | --:       | :--    | --:           | --:           | --:           | --:           |
| lan, apply 100 µs/call          | 10,000/s  | 1 s    | 9,979, 3.1    | 9,998, 3.1    | 10,021, 3.1   | 10,000, 3.1   |
| lan, apply 100 µs/call          | saturated | 1 s    | 242,823, 1.2  | 248,409, 1.2  | 249,014, 1.2  | 245,379, 1.2  |
| lan, apply 500 µs/call          | 10,000/s  | 1 s    | 9,969, 3.5    | 10,001, 3.5   | 10,014, 3.4   | 9,998, 3.5    |
| lan, apply 500 µs/call          | saturated | 1 s    | 166,600, 1.7  | 141,934, 2.1  | 153,744, 2.0  | 143,668, 2.0  |
| lan, apply 2 ms/call            | 10,000/s  | 1 s    | 9,950, 6.9    | 9,991, 7.1    | 10,013, 6.9   | 9,996, 6.9    |
| lan, apply 2 ms/call            | saturated | 1 s    | 74,384, 3.7   | 52,394, 5.1   | 51,743, 5.2   | 51,553, 5.2   |
| lan, 100 µs/call + 10 µs/entry  | 10,000/s  | 1 s    | 9,978, 3.0    | 10,000, 3.1   | 10,018, 3.1   | 9,999, 3.0    |
| lan, 100 µs/call + 10 µs/entry  | saturated | 1 s    | 62,380, 4.5   | 72,884, 4.4   | 73,179, 3.6   | 73,097, 3.6   |
| cloud, apply 500 µs/call        | 10,000/s  | 1 s    | 9,955, 5.6    | 9,993, 5.6    | 10,017, 5.6   | 9,998, 5.6    |
| cloud, apply 500 µs/call        | saturated | 1 s    | 67,865, 5.4   | 68,067, 4.0   | 69,312, 5.8   | 69,026, 4.1   |
| lan, apply 2 ms/call            | 10,000/s  | 5 s    | 9,945, 6.8    | 10,001, 7.0   | 9,984, 7.0    | 10,002, 6.7   |
| lan, 100 µs/call + 10 µs/entry  | 10,000/s  | 5 s    | 9,977, 2.9    | 10,000, 3.1   | 10,006, 3.1   | 10,000, 3.0   |

The paced rows kept their rate and their p99 in every phase. Among the saturated rows, the
100 µs row stayed within 3%. The others changed their rate at the cut and kept the new one
after the catch-up. Run by run (`results/tables.md`), these are the two patterns of writes per
call described in the apply-lag report, each with its own p50:

- With 2 ms calls, 3 of 5 runs ran at 74,000 to 76,000 writes/s (p50 3.4 ms) before the cut and
  at 51,000 to 53,000 (p50 5.0 to 5.1 ms) by the end; the other 2 ran at the slower rate
  throughout.
- With 500 µs calls, 4 runs ran at 163,000 to 168,000 writes/s (p50 1.54 to 1.57 ms) before the
  cut, and 3 of them dropped to 132,000 to 143,000 (p50 1.90 to 1.95 ms) at it.
- With 10 µs per entry, 4 runs ran at 60,000 to 62,000 writes/s (p50 4.1 to 4.3 ms) before the
  cut, and 3 of them rose to 73,000 to 76,000 (p50 3.4 to 3.5 ms) by the end.

The cut disturbs when writes commit, which can move the closed loop of 256 waiting clients from
one pattern to the other, in either direction. The rate during catch-up matched the rate after
it in these runs, so the catch-up itself did not slow the leader.

## What these numbers do not show

- **A partition that drops packets.** The cut fails `stream_append()` before it reads the
  request stream, as a refused connection does. The replication task then opens the stream
  again at once, without a backoff: in one 1 s outage, a counter added for that run, not part
  of this change, counted about 180,000 refused opens from node 0 to node 2. The leader's
  writes/s did not drop on this 16-core machine. A partition that drops packets would instead
  hold each open until a transport timeout.
- **A catch-up through a snapshot.** With `snapshot_policy` `Never` the leader keeps every
  entry. With the default policy it purges logs that a snapshot covers, and a follower further
  behind would install a snapshot instead of applying the entries it missed.
- **Real disk and payload.** Every cost is a fixed sleep, every write is empty, and the
  transfer of 249,000 entries costs no bandwidth.
- **Larger clusters and remote nodes.** One of two followers is cut off, and all three nodes
  share one process and one runtime.
- **The effect of a larger apply budget.** Every call took the budget of 1,024 entries; a
  different budget needs a code change and was not tried.

## Reproduce

From the repository root:

```sh
cargo build --release --manifest-path benchmarks/minimal/Cargo.toml --bin bench
BENCH=benchmarks/minimal/target/release/bench
LAN="--append-delay-us 20 --flush-delay-us 200 --network-delay-us 100 --apply-read-delay-us 50"
CUT="--cut-follower-after-ms 1000 --cut-follower-ms 1000"

# lan, apply 2 ms/call, 10,000 writes/s, 1 s outage
$BENCH --latency -c 256 -n 49920 --write-rate 10000 $LAN --apply-io-delay-us 2000 $CUT \
  --raft-config '{"snapshot_policy":"Never"}'
# the same, saturated
$BENCH --latency -c 256 -n 339968 $LAN --apply-io-delay-us 2000 $CUT \
  --raft-config '{"snapshot_policy":"Never"}'
# lan, apply 100 µs/call, 10,000 writes/s, with a 10 ms replication backoff
$BENCH --latency -c 256 -n 49920 --write-rate 10000 $LAN --apply-io-delay-us 100 $CUT \
  --raft-config '{"snapshot_policy":"Never","backoff":"10ms"}'
```

Each run prints a JSON line with a `catch_up` object: the follower's state at the cut, at the
reconnection and when it applied the target, the times in `catch_up.catch_up`, its `apply()`
calls in `apply_calls_until_caught_up`, and the leader's writes per phase in `writes`. The `n`
of a run is its rate times 5 s (9 s with a 5 s outage), rounded down to a multiple of 256. A
saturated run uses the row's saturated rate in the apply-lag report: 247,000, 169,000, 68,000,
65,000 and 69,000 writes/s, in the order of the tables.
