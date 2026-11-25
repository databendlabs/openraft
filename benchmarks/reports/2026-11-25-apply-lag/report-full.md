# Does apply lag commit on main? Leader stage latencies of paced and saturated writes

## Summary

- Commit does not wait for apply. Up to commit, a write took 640 to 770 µs on the leader (the
  sum of its stage p50s) in every `lan` row, whatever the state machine cost and the load, and
  2.7 to 2.9 ms in the `cloud` rows.
- With a cost per `apply()` call of 500 µs or more, the time from commit to apply was 1.4 to 1.9
  calls at 10,000 writes/s, at 50,000 writes/s and at saturation: 0.71 to 0.77 ms with 500 µs
  calls, and 2.9 to 3.7 ms with 2 ms calls. It does not grow with load, because the worker
  applies the commits queued during a call in one following call: the saturated 2 ms row
  averaged 215 entries per apply command.
- A cost per entry makes each call longer as the load rises, so the time from commit to apply
  grows with load. With 10 µs per entry it was 0.35 ms at 10,000 writes/s, 0.87 ms at 50,000
  writes/s, and 3.2 ms at saturation, where an apply command carried 179 entries on average.
- How far apply lags commit depends on how long one `apply()` call takes. With 100 µs calls,
  commit to apply took 0.24 to 0.37 ms, less than the commit; with 500 µs calls, 0.71 to
  0.77 ms, about as long as the commit's 0.66 to 0.73 ms. With calls of about 2 ms, it was 79%
  to 86% of the leader's latency per entry: an entry committed during a call waits for that call
  to return, then for its own call, because the worker runs one `apply()` call at a time.
- Saturated throughput with 256 clients matched 256 writes in flight divided by the leader's
  latency per entry in every row, within 4%: 247k writes/s with 100 µs calls, 169k with 500 µs
  calls, and 68k with 2 ms calls. In this closed loop a slower state machine lowers throughput
  through latency. More clients would make the batches larger; that was not measured.

## What was measured

The target is `benchmarks/minimal` at `51592651` (`ups/main`): three voters in one process, an
in-memory log store, empty `client_write()` calls, and a network of in-process function calls.
Every delay is a sleep on a blocking-pool thread. The release build has the default
`runtime-stats` feature, which records per-entry stage timestamps on the leader.

The delays reuse the profiles of the [v0.10.0 vs main report](../2026-11-17-v0.10.0-vs-main/report-full.md),
in µs:

| profile | append | flush | network, one way | apply read | apply I/O per call |
| :--     | --:    | --:   | --:              | --:        | --:                |
| `lan`   | 20     | 200   | 100              | 50         | varies, below      |
| `cloud` | 50     | 1,000 | 500              | 100        | 500                |

The `lan` rows vary the state machine:

| state machine                         | flags                                                | stands for |
| :--                                   | :--                                                  | :--        |
| apply 100 µs/call                     | `--apply-io-delay-us 100`                            | a fast KV store |
| apply 500 µs/call                     | `--apply-io-delay-us 500`                            | a slower store |
| apply 2 ms/call                       | `--apply-io-delay-us 2000`                           | a store that makes each batch durable on slow storage |
| apply 100 µs/call + 10 µs/entry       | `--apply-io-delay-us 100 --apply-entry-delay-us 10`  | work for each entry, such as one key write each; it caps the state machine below 100,000 entries/s |

Every row runs 256 clients, each with one write in flight. The paced rows add
`--write-rate 10000` or `--write-rate 50000` and write twice the rate, so they last about 2 s.
The saturated rows are unpaced; one calibration run per state machine set their write count to
about 2 s of writes.

## Method

- Machine: Apple M3 Max (16 cores), macOS 14.5, rustc 1.96.0-nightly (2026-02-28). One client
  worker thread, 16 server worker threads, 1,024 warmup writes, the bench's default `Config`.
- Each row ran 5 times. Every round ran all rows once, in an order shuffled with seed 812, so a
  change of machine load affects all rows alike. Tables show medians.
- The machine was not idle: the 1-minute load average at the start of a run ranged from 6.3 to
  26.0 (median 14.8). A loaded machine wakes the blocking-pool sleeps late, so every delay costs
  more than its flag.
- Client latency is `request_p50_ns` and `request_p99_ns` from the JSON line the bench prints.
  In paced rows it starts at the write's scheduled arrival, so it includes the pacing delay;
  the `arrival no-op` rows measure that delay alone with `--arrival-noop`. They come from a
  second pass of the grid, described under "What these numbers do not show".
- Stage latencies are the `Log Stage Latencies` table the leader prints at exit. They are per
  entry. "Committed" is when `RaftCore` starts `SaveCommittedAndApply` for the entry, and
  "applied" is when `RaftCore` handles the state machine worker's result, so committed→applied
  includes the apply queue, the log read, the `apply()` call and the hand-offs. The leader keeps
  the timestamps of its last 1,024 log ranges per stage (`Config::log_stage_capacity`), so the
  histograms cover the end of each run: 9,530 to 114,688 entries, the "stage samples" column of
  `results/tables.md`.
- Entries per apply command is the number of writes divided by the `SaveCommittedAndApply`
  commands the leader sent after warmup. The worker merges the commands queued when it starts a
  batch into one `apply()` call, so an `apply()` call carries at least this many entries on
  average.

## Results

Complete tables, with run ranges and stage p99s, are in `results/tables.md`.

### Throughput and client latency

| state machine                   | load      | writes/s | client p50 µs | client p99 µs |
| :--                             | --:       | --:      | --:           | --:           |
| lan, apply 100 µs/call          | 10,000/s  |    9,992 |         1,737 |         3,113 |
| lan, apply 100 µs/call          | 50,000/s  |   49,951 |         1,753 |         3,178 |
| lan, apply 100 µs/call          | saturated |  246,646 |         1,032 |         1,229 |
| lan, apply 500 µs/call          | 10,000/s  |    9,987 |         2,523 |         3,637 |
| lan, apply 500 µs/call          | 50,000/s  |   49,949 |         2,556 |         3,604 |
| lan, apply 500 µs/call          | saturated |  169,419 |         1,524 |         1,704 |
| lan, apply 2 ms/call            | 10,000/s  |    9,972 |         5,046 |         6,750 |
| lan, apply 2 ms/call            | 50,000/s  |   49,840 |         5,571 |        12,321 |
| lan, apply 2 ms/call            | saturated |   68,320 |         3,506 |         5,177 |
| lan, 100 µs/call + 10 µs/entry  | 10,000/s  |    9,991 |         1,917 |         3,015 |
| lan, 100 µs/call + 10 µs/entry  | 50,000/s  |   49,940 |         2,654 |         4,162 |
| lan, 100 µs/call + 10 µs/entry  | saturated |   65,239 |         4,063 |         4,588 |
| cloud, apply 500 µs/call        | 10,000/s  |    9,978 |         4,522 |         5,571 |
| cloud, apply 500 µs/call        | saturated |   69,026 |         3,736 |         4,030 |
| arrival no-op                   | 10,000/s  |    9,994 |         1,245 |         2,327 |
| arrival no-op                   | 50,000/s  |   49,966 |         1,262 |         2,359 |

Every paced row except the 2 ms row at 50,000 writes/s reached its rate in every run, and no
write failed. The no-op rows show that pacing alone adds about 1.25 ms to the p50 and 2.3 ms to
the p99 of a paced write: the bench releases paced writes on 1 ms timer ticks. In paced rows,
the leader's proposed→applied latency below is the better measure of a write.

The 2 ms row at 50,000 writes/s ran at the limit of 256 clients: with about 5.6 ms per write,
256 writes in flight allow about 46,000 writes/s once the pacing delay is counted. One of its
five runs reached only 44,000 writes/s, and the row's client p99 was 12.3 ms, against 6.8 ms at
10,000 writes/s.

### Leader stage latencies

Latencies in µs, per entry, on the leader. "Up to commit" is the sum of the p50s of the four
stages before commit, not a percentile of the sum. "After commit" is committed→applied as a share
of proposed→applied, at p50.

| state machine                   | load      | up to commit | committed→applied p50 / p99 | proposed→applied p50 / p99 | after commit | entries per apply command |
| :--                             | --:       | --:          | --:                         | --:                        | --:          | --:                       |
| lan, apply 100 µs/call          | 10,000/s  |          664 |                   239 / 299 |                923 / 1,022 |          26% |                        13 |
| lan, apply 100 µs/call          | 50,000/s  |          693 |                   239 / 305 |                955 / 1,160 |          25% |                        59 |
| lan, apply 100 µs/call          | saturated |          746 |                   372 / 499 |              1,074 / 1,395 |          35% |                       254 |
| lan, apply 500 µs/call          | 10,000/s  |          655 |                 712 / 1,467 |              1,417 / 2,838 |          50% |                        12 |
| lan, apply 500 µs/call          | 50,000/s  |          679 |                 722 / 1,292 |              1,417 / 2,014 |          51% |                        63 |
| lan, apply 500 µs/call          | saturated |          731 |                 768 / 1,010 |              1,517 / 2,375 |          51% |                       133 |
| lan, apply 2 ms/call            | 10,000/s  |          655 |               3,176 / 4,837 |              3,798 / 5,177 |          84% |                        12 |
| lan, apply 2 ms/call            | 50,000/s  |          703 |               3,700 / 5,121 |              4,313 / 5,872 |          86% |                        62 |
| lan, apply 2 ms/call            | saturated |          749 |               2,866 / 5,058 |              3,615 / 5,674 |          79% |                       215 |
| lan, 100 µs/call + 10 µs/entry  | 10,000/s  |          642 |                   352 / 505 |                983 / 1,260 |          36% |                        10 |
| lan, 100 µs/call + 10 µs/entry  | 50,000/s  |          689 |                 872 / 1,527 |              1,682 / 2,517 |          52% |                        55 |
| lan, 100 µs/call + 10 µs/entry  | saturated |          764 |               3,229 / 3,912 |              3,843 / 4,823 |          84% |                       179 |
| cloud, apply 500 µs/call        | 10,000/s  |        2,711 |                 810 / 1,320 |              3,379 / 3,965 |          24% |                        12 |
| cloud, apply 500 µs/call        | saturated |        2,893 |                 960 / 1,271 |              3,835 / 4,562 |          25% |                       256 |

**Commit.** In the `lan` rows, the time up to commit was 642 to 764 µs. It rose by 12% to 19%
from 10,000 writes/s to saturation, and it did not depend on the state machine: the 2 ms rows
committed as fast as the 100 µs rows. Of that time, persisted→committed, the remaining wait for
a follower to append, flush and answer, took 342 to 402 µs at p50.

**A cost per call.** Committed→applied stayed near a fixed multiple of the call: 1.4 to 1.5 calls
with 500 µs calls, and 1.4 to 1.9 calls with 2 ms calls, from 10,000 writes/s to saturation. The
load changed the batch instead: with 2 ms calls, an apply command carried 12 entries at 10,000
writes/s and 215 at saturation. With 100 µs calls the fixed costs dominate: the 50 µs log read
and the hand-offs make committed→applied 2.4 calls at the paced rates. The saturated 2 ms row
had the lowest committed→applied of its three rows; I did not trace why.

**A cost per entry.** With 10 µs per entry, a call lasts 100 µs plus 10 µs for each of its
entries, and larger batches make it longer. The per-entry work of an average apply command grew
from 0.1 ms at 10,000 writes/s to 0.55 ms at 50,000 writes/s and 1.8 ms at saturation, and
committed→applied grew with it, from 352 to 872 and 3,229 µs: 1.3 to 1.8 times such a call.
At saturation the state machine still had room: at 65,000 writes/s its delays added up to about
0.7 s per second.

**cloud.** A 1 ms flush and 0.5 ms network hops make the commit 2.7 to 2.9 ms, three times the
apply. Committed→applied, 810 to 960 µs, is 1.6 to 1.9 calls of 500 µs, a little more than the
1.4 to 1.5 calls of the `lan` rows.

## What an earlier start of apply could save

An application could start applying an entry when it commits, for example from
`RaftLogStorage::save_committed()`, instead of when OpenRaft calls `apply()`. It still has to do
the entry's own apply work, so at most it saves committed→applied minus one call. At p50 in
these rows that is 0.14 to 0.27 ms with 100 µs calls, 0.21 to 0.27 ms with 500 µs calls, and
0.87 to 1.7 ms with 2 ms calls. Most of the larger saving is the wait for the call in progress,
which an application only avoids if its state machine can apply two batches at once.

## What these numbers do not show

- **Catch-up.** Every follower stays current. A follower that returns after an outage and
  applies a long backlog is not measured.
- **Real disk and payload.** Every cost is a fixed sleep, and every write is empty. A real
  store's apply cost grows with the bytes of a batch, and real I/O contends for the device.
- **The backlog of committed but unapplied entries.** `--measure-apply` records it, but in three
  interleaved pairs it lowered the saturated 2 ms row from 66,000 to 75,000 writes/s to 51,000
  to 61,000, and raised that row's committed→applied p50 from 2.8 to 3.0 ms to 4.4 to 4.5 ms.
  The paced 100 µs row did not change. So this report does not use it. In this closed loop the
  leader's backlog cannot exceed 256 entries, one per client.
- **Overload.** Each client waits for its write, so the offered load never exceeds what the
  cluster completes, and a queue that grows without bound cannot appear. `--open-loop` can
  measure that.
- **Followers.** The stage latencies are the leader's. Followers apply on their own.
- **Small differences.** A second pass of the grid, run with `--measure-apply` at a load average
  of 7.3 to 32.0, measured stage p50s 12% to 35% higher in the paced rows. In the paced 100 µs
  row, three interleaved pairs with and without the flag measured nearly the same
  committed→applied p50, 228 to 236 µs, so most of that difference is machine load. To measure a
  change, compare it with its base revision on the same machine at the same time.

## Reproduce

From the repository root:

```sh
cargo build --release --manifest-path benchmarks/minimal/Cargo.toml --bin bench
BENCH=benchmarks/minimal/target/release/bench
LAN="--append-delay-us 20 --flush-delay-us 200 --network-delay-us 100 --apply-read-delay-us 50"

# lan, apply 2 ms/call, 10,000 writes/s
$BENCH --latency -c 256 -n 20000 --write-rate 10000 $LAN --apply-io-delay-us 2000
# lan, apply 2 ms/call, saturated
$BENCH --latency -c 256 -n 102912 $LAN --apply-io-delay-us 2000
# pacing control
$BENCH --latency -c 256 -n 20000 --write-rate 10000 --arrival-noop
```

Each run prints its stage latencies to stderr after `Final:`, and a JSON line to stdout with
`operations_per_second`, `request_p50_ns`, `request_p99_ns` and `command_counts`. The `n`
column of `results/tables.md` gives the write count of every row.
