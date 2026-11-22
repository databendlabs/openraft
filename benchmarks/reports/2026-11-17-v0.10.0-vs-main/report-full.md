# Openraft v0.10.0 vs main: write benchmark with simulated per-phase delays

## Summary

- With realistic delays in every phase and a state machine that applies a batch in 100 µs or
  less, main and v0.10.0 perform the same. In the same-datacenter profile, 256 clients reach
  215k writes/s on both (+0.3%). In the cross-AZ cloud profile they reach 62k and 63k writes/s
  (+1.8%). All of these differences are within run-to-run noise.
- When the state machine is the slowest phase, main is much faster. Same-datacenter delays with
  a 500 µs apply give 231k → 509k writes/s at 1024 clients (2.2×). With a 1 ms apply the result
  is 77k → 365k writes/s (4.8×), and p99 drops from 27.6 ms to 3.8 ms.
- The worst case for v0.10.0 is fast commits plus a slow state machine. With no disk or network
  delay and 100 µs per `apply()` call, 256 clients get 8.8k → 717k writes/s (81×). With 50 µs
  per apply log read, they get 15.4k → 969k writes/s (63×).
- The gain comes from the state-machine worker. v0.10.0 makes one `apply()` call for every
  apply command RaftCore sends, and under load those commands carry only 1 or 2 entries. main
  merges the queued commands into one call (commit `b0656ad1`) and reads the next batch while
  the current one applies (commit `b91ba61d`).
- No row with a single client changes beyond noise. Without delays, main is 0.7% to 4.8% slower
  in throughput, but every one of those rows has overlapping min-max ranges, so that difference
  is not established.

## What was compared

| build   | commit     | description                                                          |
| :--     | :--        | :--                                                                  |
| v0.10.0 | `6a6c8e71` | tag `v0.10.0`                                                        |
| main    | `b91ba61d` | `ups/main`, "perf: core: read the next apply batch while the state machine applies" |

main is 37 commits ahead of v0.10.0. The commits that touch the write path are:

- `b0656ad1` perf: core: merge queued apply commands in the state-machine worker
- `b91ba61d` perf: core: read the next apply batch while the state machine applies
- `5c3ad1bc` perf: core: reserve 64 client responder slots up front
- `30c947ef` perf: engine: reduce initial queue reserves
- the cancellation changes (`7628ad40`, `05043055`, `e2d8446d`, `44dc16e7`, and others), which
  wrap internal waits in cancellable futures

### Benchmark harness

The target is `benchmarks/minimal`: three voters in one process, an in-memory log store, an
empty state machine, and a network made of in-process function calls. Writes are empty
`client_write()` calls, and latency is measured per write.

Both builds run the same harness, so any difference comes from the openraft crate:

1. The harness at v0.10.0 can delay the log append, the flush, and the network, but it has no
   apply-path delays; main added those in `af330519`. I copied main's `src/store.rs` and
   `src/network.rs` into the v0.10.0 tree and added the `--apply-read-delay-us` and
   `--apply-io-delay-us` flags to its `bench.rs`, using main's code. After this, the library
   files are byte-identical in both trees. The write loop in `bench.rs` was already the same.
2. In both trees I changed the append, flush, and network delays from `tokio::time::sleep` to
   the harness's existing `blocking_delay()`, which already implemented the apply delays.
   Tokio's timer rounds up to a 1 ms tick. Before the change, a "100 µs" one-way network delay
   produced a 6.1 ms p50 for one client, and a "100 µs" flush produced 1.6 ms. After the
   change the same settings give 0.58 ms and 0.18 ms. `blocking_delay()` sleeps on a
   blocking-pool thread.

The v0.10.0 harness patch is in `results/harness-v0.10.0.diff`. The main harness needed only
change 2, and after both patches `src/store.rs` and `src/network.rs` are identical in the two
trees, so the main side is rebuilt by copying those two files from the patched v0.10.0 tree.
Neither patch is committed to either revision.

## Delay model

A write passes through five simulated phases. Each delay is a fixed cost per call:

| phase       | flag                    | what it stands for                                         |
| :--         | :--                     | :--                                                        |
| append      | `--append-delay-us`     | submitting the log write, such as `write()` into a log file  |
| flush       | `--flush-delay-us`      | fsync; completes asynchronously, several can be in flight  |
| network     | `--network-delay-us`    | one-way AppendEntries delay, paid on the request and on the response |
| apply read  | `--apply-read-delay-us` | the log read that loads each apply batch (`entries_stream()`) |
| apply I/O   | `--apply-io-delay-us`   | state-machine write per `apply()` call; client responses wait for it |

The profiles below give the delays in µs, in the order append / flush / network one-way /
apply read / apply I/O:

| profile      | delays (µs)              | models                                                       |
| :--          | :--                      | :--                                                          |
| `none`       | 0 / 0 / 0 / 0 / 0        | Openraft's own overhead only (control)                       |
| `append`     | 20 / 0 / 0 / 0 / 0       | one phase at a time, using the `lan` value                   |
| `flush`      | 0 / 200 / 0 / 0 / 0      | same                                                         |
| `network`    | 0 / 0 / 100 / 0 / 0      | same                                                         |
| `apply-read` | 0 / 0 / 0 / 50 / 0       | same                                                         |
| `apply-io`   | 0 / 0 / 0 / 0 / 100      | same                                                         |
| `lan`        | 20 / 200 / 100 / 50 / 100 | same datacenter: NVMe fsync of about 200 µs, 200 µs RTT, a fast KV state machine |
| `cloud`      | 50 / 1000 / 500 / 100 / 500 | cross-AZ: cloud block storage fsync of 1 ms, 1 ms RTT      |
| `lan-sm500`  | 20 / 200 / 100 / 50 / 500 | `lan` with a slower state machine                           |
| `lan-sm1000` | 20 / 200 / 100 / 50 / 1000 | `lan` with a state machine that fsyncs each batch           |

## Method

- Machine: Apple M3 Max (16 cores), macOS 14.5, rustc 1.96.0-nightly (2026-02-28), release
  build with the default `runtime-stats` feature.
- Each run uses 1 client worker thread and 16 server worker threads, 1,024 warmup writes, and
  the benchmark's default `Config`: election timeout 200 to 2000 ms, `max_payload_entries` 1024.
- Operation counts were calibrated per row so that one run lasts about 1.5 s. The main build
  was used for calibration, except for the `lan-sm*` rows, which used v0.10.0. Both builds use
  the same count within a row.
- Each row ran 7 times per build. The two binaries ran back to back, and their order swapped on
  every round, so changes in machine load affect both builds alike.
- Tables show medians. A `~` before a change means the min-max ranges of the two builds
  overlap, so that change is within noise.
- The machine was not idle: the 1-minute load average ranged from 6.6 to 16.1 (median 8.9)
  because of an IDE and other applications. The tail-latency numbers are the most affected.
- Each run used `/usr/bin/time -l` for CPU time and max RSS. Per-stage latency, apply command
  counts, and entries per apply come from the `runtime-stats` output the bench prints at exit.

## Results

The tables use "0.10" for v0.10.0. The complete tables, including per-stage latency for every
row, are in `results/tables.md`.

### Control: no delays

| clients | writes/s 0.10 | writes/s main | change | p50 µs 0.10 | p50 µs main | p99 µs 0.10 | p99 µs main |
| --: | --: | --: | --: | --: | --: | --: | --: |
| 1    | 28,779    | 27,394    | ~-4.8% | 33  | 34  | 62  | 70  |
| 64   | 834,261   | 827,948   | ~-0.8% | 73  | 75  | 172 | 172 |
| 256  | 1,467,309 | 1,457,684 | ~-0.7% | 166 | 171 | 306 | 332 |
| 1024 | 2,113,595 | 2,015,603 | ~-4.6% | 476 | 492 | 837 | 882 |

Every change in this table is within noise. main is slightly lower in all four medians, by
0.7% to 4.8% in throughput and 2.6% to 3.3% in p50. That pattern may be a small real cost, for
example from the cancellable-future wrappers, but this run cannot separate it from noise.
Measuring it would need an idle machine and more runs.

### One phase at a time

| profile | clients | writes/s 0.10 | writes/s main | change | p50 µs 0.10 | p50 µs main | p99 µs 0.10 | p99 µs main |
| :-- | --: | --: | --: | --: | --: | --: | --: | --: |
| append 20 µs      | 1   | 8,395   | 8,487   | ~+1.1%     | 114    | 112 | 199    | 177 |
| append 20 µs      | 256 | 832,910 | 817,593 | ~-1.8%     | 300    | 309 | 474    | 560 |
| flush 200 µs      | 1   | 3,196   | 3,178   | ~-0.6%     | 311    | 312 | 355    | 350 |
| flush 200 µs      | 256 | 430,697 | 427,409 | ~-0.8%     | 597    | 602 | 820    | 829 |
| network 100 µs    | 1   | 1,871   | 1,867   | ~-0.2%     | 578    | 575 | 664    | 663 |
| network 100 µs    | 256 | 423,016 | 421,651 | ~-0.3%     | 619    | 616 | 743    | 865 |
| apply read 50 µs  | 1   | 8,231   | 8,300   | ~+0.8%     | 119    | 118 | 162    | 160 |
| apply read 50 µs  | 256 | 15,400  | 968,826 | **+6191%** | 17,302 | 252 | 22,658 | 431 |
| apply I/O 100 µs  | 1   | 5,381   | 5,365   | ~-0.3%     | 183    | 184 | 222    | 227 |
| apply I/O 100 µs  | 256 | 8,824   | 717,443 | **+8031%** | 30,293 | 332 | 38,784 | 589 |

Delays in the append, flush, and network phases give the same results on both builds. A delay
in either apply phase, with 256 clients, collapses v0.10.0 to 9k to 15k writes/s, while main
stays at 0.7M to 1.0M writes/s. In all seven v0.10.0 runs of these rows, the throughput
stayed between 8.5k and 16.6k writes/s, so the collapse happens every time.

### All phases delayed

| profile | clients | writes/s 0.10 | writes/s main | change | p50 µs 0.10 | p50 µs main | p99 µs 0.10 | p99 µs main |
| :-- | --: | --: | --: | --: | --: | --: | --: | --: |
| lan   | 1    | 1,078   | 1,086   | ~+0.7% | 917   | 916   | 1,052 | 1,035 |
| lan   | 64   | 62,865  | 64,255  | ~+2.2% | 987   | 981   | 1,282 | 1,206 |
| lan   | 256  | 214,639 | 215,220 | ~+0.3% | 1,193 | 1,190 | 1,482 | 1,425 |
| lan   | 1024 | 707,687 | 688,243 | ~-2.7% | 1,445 | 1,481 | 1,768 | 1,829 |
| cloud | 1    | 244     | 249     | ~+2.0% | 3,995 | 3,984 | 6,271 | 5,319 |
| cloud | 64   | 16,051  | 15,662  | ~-2.4% | 3,984 | 4,008 | 4,354 | 6,182 |
| cloud | 256  | 61,605  | 62,724  | ~+1.8% | 4,010 | 3,998 | 7,013 | 5,896 |
| cloud | 1024 | 207,219 | 200,984 | ~-3.0% | 5,276 | 5,267 | 5,766 | 7,696 |

These rows show no measurable difference. The p99 values move by up to ±42%, but in both
directions and always within the overlapping ranges of a noisy machine.

For one client in the `lan` profile, the leader's log-stage latencies (p50) explain the 0.92 ms
total. Persisting takes 0.31 ms (20 µs append, 200 µs flush, plus thread hand-off). Committing
takes 0.36 ms (two network hops, plus the follower's append and flush). Applying takes 0.24 ms
(50 µs read, 100 µs I/O, plus hand-off). These numbers match on both builds.

### All phases delayed, slower state machine

| profile | clients | writes/s 0.10 | writes/s main | change | p50 µs 0.10 | p50 µs main | p99 µs 0.10 | p99 µs main |
| :-- | --: | --: | --: | --: | --: | --: | --: | --: |
| lan-sm500  | 1    | 706     | 705     | ~-0.2%      | 1,409  | 1,412 | 1,583  | 1,519 |
| lan-sm500  | 64   | 42,041  | 43,022  | ~+2.3%      | 1,490  | 1,481 | 1,948  | 1,661 |
| lan-sm500  | 256  | 60,768  | 158,020 | ~+160.0%    | 2,259  | 1,617 | 10,009 | 1,874 |
| lan-sm500  | 1024 | 231,492 | 508,664 | **+119.7%** | 3,099  | 1,959 | 9,637  | 2,631 |
| lan-sm1000 | 1    | 487     | 486     | ~-0.2%      | 2,052  | 2,055 | 2,186  | 2,251 |
| lan-sm1000 | 64   | 28,136  | 30,080  | ~+6.9%      | 2,158  | 2,129 | 2,790  | 2,292 |
| lan-sm1000 | 256  | 90,682  | 97,808  | ~+7.9%      | 2,737  | 2,589 | 4,173  | 3,122 |
| lan-sm1000 | 1024 | 76,556  | 365,302 | **+377.2%** | 10,972 | 2,797 | 27,636 | 3,779 |

In these rows v0.10.0 becomes unstable while main does not. The table below lists the minimum
and maximum throughput of the 7 runs, in thousands of writes per second:

| row               | v0.10.0      | main         |
| :--               | --:          | --:          |
| lan-sm500, 256    | 39.0 – 153.5 | 141.0 – 159.3 |
| lan-sm500, 1024   | 116.0 – 462.6 | 478.1 – 526.0 |
| lan-sm1000, 256   | 45.6 – 114.8 | 94.3 – 105.6 |
| lan-sm1000, 1024  | 53.1 – 139.7 | 322.9 – 376.0 |

The `~` on the lan-sm500 256-client row comes from one fast v0.10.0 run (153.5k). In the
other six runs, v0.10.0 was slower than main's slowest run. The behavior looks like a tipping
point: some v0.10.0 runs keep up and others fall behind, and the runs that fall behind are much
slower. I did not trace how this develops within a single run.

## Why main is faster: the apply path

RaftCore sends one `SaveCommittedAndApply` command each time the commit index advances. The
two builds handle these commands differently:

- **v0.10.0**: the state-machine worker runs each command alone, with one `entries_stream()`
  read and one `apply()` call. The fixed cost of each call is paid once per command, however
  few entries the command holds.
- **main**: the worker takes all queued apply commands and applies them as one range
  (`b0656ad1`). It also reads the next range while the current one runs through `apply()`
  (`b91ba61d`), so a batch costs the larger of its read and its I/O instead of their sum.

The apply I/O 100 µs row with 256 clients shows the effect:

| metric                                  | v0.10.0   | main     |
| :--                                     | --:       | --:      |
| apply commands from RaftCore (warmup included) | 850,975 | 15,541 |
| entries per apply, p50 and p90          | 1, 2      | 69, 132  |
| leader committed→applied p50 / p99 (µs) | 30,921 / 38,256 | 254 / 352 |
| CPU time per 1,000 writes (ms)          | 106.5     | 2.9      |

With no disk or network delay, the commit index advances on almost every follower response.
Each command then carries 1 or 2 entries, and v0.10.0 pays about 150 µs (100 µs of I/O plus
the thread hand-off) for each one. That caps it near 9k writes/s. Entries pile up between
commit and apply, and that queue makes up the whole 30 ms p50. Every other stage stays at or
below 10 µs. The CPU cost also rises, because every apply call wakes a blocking-pool thread.

With realistic disk and network delays, commits advance less often and each command already
carries tens to hundreds of entries. In the `lan` 256-client row, v0.10.0 received 3,247
commands for 340k writes. When the per-call apply cost stays below the interval between commit
advances, the worker never falls behind and merging has nothing to merge. That is why `lan`
and `cloud` show no change.
When the apply cost exceeds that interval, as with 500 µs or 1 ms state-machine I/O under
heavy load, v0.10.0 falls behind and main does not. In the lan-sm1000 1024-client row,
v0.10.0 needed 6,520 commands and spent 26.4 ms (p50) between commit and apply. main needed
2,131 commands and spent 2.2 ms.

The rule of thumb: main helps when one `apply()` call costs more than the time between two
commit advances on the leader. The cost of one call includes the log read, the state-machine
write, and any fsync. A state machine that makes each batch durable, applies with
synchronous I/O, or reads its logs from disk is likely to cross that line under load.

## What these numbers do not show

- **A real deployment.** Each delay is a fixed sleep per call. Real fsync and network costs vary
  and grow with payload size, and real I/O contends for devices. Every write here is empty.
- **Per-entry costs.** All apply costs here are per call, so merging spreads them across every
  entry in the batch. A cost per entry, such as one KV put per entry, would not shrink by
  merging. On main it would still overlap with the log read.
- **Network scaling.** All three nodes share one process and one 16-thread runtime. CPU is
  shared, and the "network" adds no serialization cost.
- **Reads, snapshots, and leader changes.** This benchmark measured writes only.
- **Memory.** In the two single-phase apply rows, main's max RSS was 81 to 84 MB against
  v0.10.0's 53 to 56 MB. In every other row the two builds were within 3 MB. I did not
  investigate the cause; a likely one is main writing 63 to 81 times as many entries per
  second before log purging catches up.
- **Small regressions.** The noise on this machine, with a load average of 6.6 to 16.1, hides
  changes of a few percent. The control rows lean slightly toward main being slower, which is
  worth re-checking on an idle machine with `--runs 15`.

## Reproduce

Run these from this directory. `$B` is a scratch directory outside the repo for the two
builds; the diff has no `a/` and `b/` prefixes, so it applies with `-p0`.

```sh
B=/tmp/bench-0.10-vs-main
git worktree add --detach $B/v0.10.0 v0.10.0
git worktree add --detach $B/main    b91ba61d
(cd $B/v0.10.0 && git apply -p0 "$OLDPWD/results/harness-v0.10.0.diff")
cp $B/v0.10.0/benchmarks/minimal/src/{store.rs,network.rs} $B/main/benchmarks/minimal/src/
(cd $B/v0.10.0/benchmarks/minimal && cargo build --release --bin bench)
(cd $B/main/benchmarks/minimal    && cargo build --release --bin bench)
```

Every row of the tables is one bench command, run with both binaries. The delay flags come
from the profile table in "Delay model", and the operation count of each row is the `n`
column of `results/tables.md`. For example, the lan-sm1000 profile with 1024 clients:

```sh
BENCH=$B/main/benchmarks/minimal/target/release/bench    # or $B/v0.10.0/...
/usr/bin/time -l $BENCH --latency -c 1024 -n 571392 --append-delay-us 20 --flush-delay-us 200 \
   --network-delay-us 100 --apply-read-delay-us 50 --apply-io-delay-us 1000
```

Run each row 7 times per build, alternating which build goes first, and take the medians of
`operations_per_second`, `request_p50_ns`, and `request_p99_ns` from the last line of output.

Remove the worktrees afterwards:

```sh
git worktree remove --force $B/v0.10.0 && git worktree remove --force $B/main
```
