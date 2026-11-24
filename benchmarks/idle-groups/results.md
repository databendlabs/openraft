# Results of the #2304 changes

These measurements follow the changes that [#2304] led to, in November 2026: the timers, the
task futures, and the heap of idle groups. The profile at ca32e84d and #2487 are follow-ups that
[#2454] tracks. The [README](README.md) describes the fields and has the baseline, 0.10.0 against
`main` at 1329ad82.
[`scripts/measure_change.py`](scripts/measure_change.py) runs comparisons like the later ones.

All runs used one Apple M3 Max (16 cores, 128 GiB) with macOS 14.5. They were release builds
with the system allocator and tokio 1.54.1 with 4 worker threads, ran the default `Config`, and
wrote nothing. A run waits 10 s after switching the timers and then measures 30 s, unless a
section says otherwise, and each row is one run. CPU is a share of one core, and memory is the
physical footprint. Repeated runs of one build differ by about 3% in CPU and by a few MiB in
memory.

## Timers

### Retiring the ticker: #2378

43643c46 ([#2378]) sends heartbeats at their deadline and retires the general ticker. Against
its parent 1f9a1ead, it removes all the CPU of idle groups with heartbeats and elections off, and
halves it with defaults. Both builds predate the fix of [#2400] below.

| Groups | Mode                 | CPU, 1f9a1ead | CPU, 43643c46 | Memory, 1f9a1ead | Memory, 43643c46 |
|-------:|----------------------|--------------:|--------------:|-----------------:|-----------------:|
|  1,000 | `default`            |           83% |           33% |          229 MiB |          231 MiB |
|  1,000 | `no-heartbeat-elect` |           72% |            0% |          230 MiB |          222 MiB |
|  1,000 | `no-tick`            |            0% |            0% |          222 MiB |          223 MiB |
|  5,000 | `default`            |          350% |          174% |        1,024 MiB |        1,034 MiB |
|  5,000 | `no-heartbeat-elect` |          326% |            0% |        1,023 MiB |          999 MiB |
|  5,000 | `no-tick`            |            0% |            0% |          982 MiB |          992 MiB |

- At 1f9a1ead, elections already run at their deadline (312e8500, [#2343]), but with heartbeats
  and elections off 1,000 groups still use 72% of a core, against 67% on 0.10.0: the ticks cost
  that CPU with nothing to time.
- Both commits send the same heartbeats: 35,867 and 36,860 AppendEntries requests a second at
  1,000 groups, 173,357 and 174,713 at 5,000.
- Memory changes by at most 2.4%. Each member runs 11 tasks at both commits: the heartbeat
  deadline notifier replaces the tick task.

### A leader's idle CPU goes to its heartbeats: #2407

`--mode no-elect` and `--mode no-heartbeat` ([#2407]) turn off one timer each. With stub
followers, built from #2407 on bc383014:

| Groups | Mode           |  CPU |  Memory | AppendEntries/s |
|-------:|----------------|-----:|--------:|----------------:|
|  1,000 | `default`      |  27% | 207 MiB |          36,908 |
|  1,000 | `no-elect`     |  31% | 211 MiB |          36,676 |
|  1,000 | `no-heartbeat` |   0% | 197 MiB |               0 |
|  5,000 | `default`      | 147% | 913 MiB |         184,872 |
|  5,000 | `no-elect`     | 146% | 912 MiB |         183,780 |
|  5,000 | `no-heartbeat` |   0% | 876 MiB |               0 |

Turning elections off leaves the CPU of the defaults, and turning heartbeats off removes all of
it. A leader has no election deadline, so the cost of elections shows only with real followers.

### Followers' election timers: #2412 and #2419

With `--followers local` ([#2412]), all three members of each group run in the process. At
71066fcc, two runs of `default` and `no-elect` and one of `no-heartbeat-elect`; no group changed
its leader:

| Groups | Mode                 |        CPU |           Memory |  AppendEntries/s |
|-------:|----------------------|-----------:|-----------------:|-----------------:|
|  1,000 | `default`            |   69%, 68% |     544, 541 MiB |   34,992, 35,198 |
|  1,000 | `no-elect`           |   58%, 62% |     527, 530 MiB |   35,199, 35,265 |
|  1,000 | `no-heartbeat-elect` |         0% |          504 MiB |                0 |
|  5,000 | `default`            | 298%, 296% | 2,529, 2,528 MiB | 186,672, 186,386 |
|  5,000 | `no-elect`           | 228%, 226% | 2,513, 2,517 MiB | 179,437, 179,146 |
|  5,000 | `no-heartbeat-elect` |         0% |        2,407 MiB |                0 |

- Turning elections off saves 8 points of a core at 1,000 groups, 12% of the CPU with defaults,
  and 70 points at 5,000, 24%. With stub followers the two modes use the same CPU, so the
  saving is the followers'. Each heartbeat moved a follower's election deadline, which woke its
  election notifier task to re-arm the timer: about 186,000 wakes a second at 5,000 groups, or
  3.8 µs per wake, including the deadline computation and the channel send.
- A whole idle group runs 25 tasks: 11 for its leader member and 7 for each follower, which also
  adds about 170 KiB. Whole groups use 68% of a core per 1,000 groups, against 27% for leaders
  with stub followers.

12e9db7e ([#2419]) wakes a notifier only when its deadline moves earlier. Against its parent
b7cb5179, with whole groups:

| Groups | Mode       | CPU, b7cb5179 | CPU, 12e9db7e |
|-------:|------------|--------------:|--------------:|
|  1,000 | `default`  |      62%, 60% |      55%, 55% |
|  1,000 | `no-elect` |           51% |           50% |
|  5,000 | `default`  |    309%, 309% |    267%, 266% |
|  5,000 | `no-elect` |          237% |          237% |

That recovers 58% of the saving of `no-elect` at 1,000 groups and 59% at 5,000. Per heartbeat,
`RaftCore` still computes the deadline and stores it in the watch channel, and the notifier
still wakes once per election timeout. With the change the groups also send 3% to 5% more
AppendEntries requests a second: 38,700 against 37,050 at 1,000 groups, and 193,600 against
188,400 at 5,000.

## Task futures

### Where an idle group's memory grew

A bisect of v0.10.0..1329ad82 with 2,000 groups in `no-tick` mode, taking the smaller memory of
two runs per commit. Runs at one commit differ by up to 4 MiB.

| Commit   | Change                                                              |  Memory | Per group |
|----------|---------------------------------------------------------------------|--------:|----------:|
| v0.10.0  |                                                                     | 361 MiB |           |
| e2d8446d | 26 commits later; 5c3ad1bc, the 17th, measured 366 MiB              | 368 MiB |  +3.6 KiB |
| 05043055 | fix: core: cancel heartbeat workers during all waits                | 391 MiB |   +12 KiB |
| 52b19f6d | fix: core: cancel blocked pending-read deadline sends               | 402 MiB |  +4.6 KiB |
| 312e8500 | change: core: start elections at their deadline instead of on ticks | 411 MiB |  +5.6 KiB |
| 1f9a1ead | 4 commits later                                                     | 414 MiB |  +1.5 KiB |
| 43643c46 | change: core: send heartbeats at their deadline instead of on ticks | 419 MiB |  +2.6 KiB |
| 1329ad82 |                                                                     | 418 MiB |           |

The other commits stay within 4 MiB of their predecessor. Four commits add 49 of the 57 MiB,
about 25 KiB per group. 05043055 and 52b19f6d wrap long-lived tasks, the two heartbeat workers
and the pending-read deadline notifier, in `cancellable()`. 312e8500 adds the election deadline
notifier task, and 43643c46 replaces the tick task with the heartbeat deadline notifier.

The size of each future that the tokio runtime spawns, in bytes, as a patched `spawn()` printed
it while one group elected its leader, built with the toolchain of `rust-toolchain`,
nightly-2026-03-01:

| Task                          | Tasks per leader member | e2d8446d | 43643c46 |
|-------------------------------|-------------------------|---------:|---------:|
| `HeartbeatWorker::run`        | 2                       |    1,448 |    7,840 |
| each deadline notifier        | 1, 2, then 3            |      696 |    4,440 |
| the tick task                 | 1, then 0               |    2,192 |          |
| `RaftCore::main`              | 1                       |    3,744 |    4,128 |
| `ReplicationCore::main`       | 2                       |    2,032 |    2,080 |
| the state-machine worker      | 1                       |    1,768 |    2,160 |
| `StepDownWatcher::watch_loop` | 1                       |    1,544 |    1,544 |
| `io_completion_forwarder`     | 1                       |      632 |      632 |
| all tasks alive while idle    |                         |   17,536 |   41,624 |

The deadline notifiers are the pending-read one, the election one from 312e8500, and the
heartbeat one from 43643c46. An `async` block keeps each future it captured by value next to
the future it awaits, so every layer of wrapping stores the inner future again
([rust-lang/rust#62958]). With a 1,025-byte inner future, `select2()` of two of them takes 4,120
bytes, `cancellable()` 3,128, and `async move { cancellable(inner).await }`, the shape of the
heartbeat worker's task, 6,264.

### Holding each future once: #2400

bc383014 ([#2400]) makes `select2()` a hand-written future that holds each future once in a
pinned field. `select3()`, `select4()` and `cancellable_by()` compose it with `map()`, and the
heartbeat worker and the notifiers map their cancellable future instead of awaiting it in an
`async move` block.

| Task                               | Before #2400, bytes | bc383014, bytes |
|------------------------------------|--------------------:|----------------:|
| `HeartbeatWorker::run`             |               7,840 |           1,336 |
| each deadline notifier             |               4,440 |             736 |
| `RaftCore::main`                   |               4,128 |           3,528 |
| all tasks of an idle leader member |              41,624 |          16,904 |

In `no-tick` mode, 1,000 groups went from 223 to 197 MiB and 5,000 groups from 1,000 to
872 MiB, within 3% of the 200 and 852 MiB of 0.10.0.

## The heap of idle groups

### Allocations: #2413

With `--count-allocs` ([#2413]), built on 71066fcc, with defaults:

| Groups | Followers | Tasks per group |    Memory |      Heap | Allocations/s | AppendEntries/s |
|-------:|-----------|----------------:|----------:|----------:|--------------:|----------------:|
|  1,000 | stub      |              11 |   208 MiB |   336 MiB |       551,417 |          36,798 |
|  1,000 | local     |              25 |   539 MiB |   910 MiB |     1,183,590 |          37,000 |
|  5,000 | local     |              25 | 2,530 MiB | 4,446 MiB |     6,110,189 |         191,013 |

Each AppendEntries request costs about 15 heap allocations on the leader's side and 17 on the
follower's: 15.0 per request with stub followers, and 32.0 with local followers. The heap
exceeds the memory, 344 KiB against 213 KiB per leader member, mostly through capacity that is
allocated and never written, as the heap profile below shows.

### Profiles at 12e9db7e

The CPU profile used samply 0.13.1 at 1 kHz on 1,000 whole groups with defaults, over a 29 s
idle window. Leaving out the samples whose leaf is a blocking wait leaves 11.1 s of busy CPU.
Shares of it, which overlap:

| Function                                            | Busy CPU |
|-----------------------------------------------------|---------:|
| `RaftCore::flush_metrics`                           |      22% |
| `RaftCore::run_engine_commands`                     |      16% |
| tokio timers                                        |      15% |
| `GroupNetworkAdapter::append_entries`, the RPC path |       8% |
| `RaftCore::refresh_election_deadline`               |       7% |

By leaf function, heap allocation and free take 10%, `memmove` 10%, clock reads 7%, and locks
5%.

- `flush_metrics` rebuilds and publishes the metrics on every loop round, and a heartbeat
  changes them on the leader. A third of its CPU is allocation and free: it builds the
  replication and heartbeat maps of `RaftMetrics` and clones them twice. Half of it is the sends
  on the watch channels: tokio's `watch::Sender::send()` notifies through `BigNotify`, eight
  mutexes, on every send.
- An allocation profile, with dhat-rs 0.3.3 as the global allocator over 10 s of 100 idle whole
  groups, counted 32.4 allocations per AppendEntries request. 16.0 of them came from
  `Engine::is_single_campaigner()`, which `refresh_election_deadline()` reached twice per round:
  each call ran `voter_ids()`, which clones the quorum sets into a `BTreeSet`, before it checked
  the node's own vote. Building and cloning the metrics maps added 12.4, and the RPC path 4.

A heap profile, with dhat-rs from the start of 200 idle leaders with stub followers, found about
437 KiB of heap per leader member. Its largest items:

| Allocation                                                                      | Per member |
|---------------------------------------------------------------------------------|-----------:|
| the six ring buffers of `LogStages` in `RuntimeStats`, never written while idle |    144 KiB |
| `PendingResponds`, four queues of 64 slots                                      |   27.5 KiB |
| tokio mpsc blocks                                                               |     24 KiB |
| the `EngineOutput` command queue, 64 slots                                      |     12 KiB |
| `LogStageHistograms`                                                            |     12 KiB |

The ring buffers, `VecDeque::with_capacity(1024)` each, account for about the 131 KiB per leader
member by which the heap exceeded the memory in the allocation counts above. base2histogram's
log-scale tables take another 26 MiB per process, in 33 allocations: a fixed cost, not one per
group.

### Heap profile at ca32e84d: #2456

For [#2456], the harness ran with dhat-rs 0.3.3 as its global allocator from the start of
`measure()`, as that issue describes, in a release build with line tables. It ran two sizes of
each case: 200 and 400 idle leaders with stub followers, and 100 and 200 idle whole groups. A
run of 400 whole groups had 8,134 elections under dhat, so the larger size is 200; no other run
had an election. Each allocation site live at the end of the window splits into bytes per node,
the slope between the two sizes, and bytes per process, the rest.

| Node                    |  Per node | Per process |
|-------------------------|----------:|------------:|
| idle leader             | 149.4 KiB |    25.5 MiB |
| member of an idle group | 135.5 KiB |    25.0 MiB |

The per-process heap is base2histogram's log-scale tables, 25.56 MiB. A `LazyLock` builds them
for all widths when the process creates its first `Histogram`, in `RuntimeStats::new()`. Per
node, in bytes:

| Allocation                                                          | Leader | Member |
|---------------------------------------------------------------------|-------:|-------:|
| tokio mpsc, the first block of each of 6 channels                   | 37,568 | 37,568 |
| the 16 histograms of `RuntimeStats`, 2,144 B each                   | 34,304 | 34,304 |
| tokio task cells: 11 per leader, 8.3 per member                     | 22,144 | 17,280 |
| tokio watch channels: 27 per leader, 21.7 per member                | 12,896 | 10,464 |
| pthread mutexes that `std::sync::Mutex` boxes, macOS only           | 12,306 |  9,366 |
| replication and heartbeat stream boxes held while idle              |  9,765 |  7,348 |
| tokio mpsc, a second block, kept once a channel carried 32 messages |  5,358 |  6,876 |
| `Box<RaftCore>`                                                     |  4,728 |  4,728 |
| other sites, each under 2 KB                                        | 13,968 | 10,823 |

- Each histogram holds 2,016 B of buckets and 128 B of region sums, and nothing can read them
  without the `runtime-stats` feature. #2487 below removes them from default builds.
- The task cells of a leader are the state-machine worker, 5,760 B; `RaftCore`, 3,712 B; two
  replication tasks, 2,176 B each; the step-down watcher, 1,792 B; two heartbeat workers,
  1,536 B each; three deadline notifiers, 896 B each; and the IO completion forwarder, 768 B.
- A watch channel takes about 480 B whatever its value, mostly for the eight `Notify` of its
  `BigNotify`. On macOS, `std::sync::Mutex` boxes a 64 B pthread mutex on its first lock, mostly
  in those `Notify`; Linux allocates nothing for it.

`tokio::sync::mpsc`, which `rt-tokio` and `rt-monoio` use, allocates the first block of a
channel when it creates it: 32 slots of the message type and a 32 B header, whatever the
capacity. `flume::bounded()` in `rt-compio` allocates only its shared state, and its queues grow
on first use. `Raft::new()` creates four of the channels, at `raft/mod.rs:573-575` and `616`,
and the state-machine worker the other two, at `sm/worker.rs:107-108`. Their message types,
from `-Zprint-type-sizes` with memstore's `TypeConfig`:

| Message, channel                        |  Size | Largest variant              |  First block |
|-----------------------------------------|------:|------------------------------|-------------:|
| `RaftMsg`, API requests                 | 240 B | `ChangeMembership`, 233 B    |      7,712 B |
| `Work`, two state-machine stages        | 200 B | `Barrier`, 200 B             | 6,432 B each |
| `sm::Command`, state-machine commands   | 192 B | `InstallFullSnapshot`, 192 B |      6,176 B |
| `InstallFullSnapshotRequest`, snapshots | 176 B |                              |      5,664 B |
| `Notification`, notifications           | 160 B | `StateMachine`, 160 B        |      5,152 B |

[#2488] boxes the large variants that are rarely sent. `Notification::StateMachine` and
`RaftMsg::ClientWrite` carry every apply result and every write, so they stay as they are.

## Changes from the profiles

### Checking a node's own vote first: #2427

01608762 ([#2427]) makes `is_single_campaigner()` check the node's own vote and
`is_campaigner()` before `voter_ids()`. With 1,000 idle whole groups, measured for 20 s after
5 s of settling, allocations fell from 1,239,778 to 618,100 a second, from 32.0 to 16.0 per
AppendEntries request. The CPU change stayed within the noise between runs.

[#2434], not merged, updated the published metrics in place. In the same setup it cut the
allocations per request from 16.0 to 4.0 and the CPU from 63% to 56% of a core, the medians of
three runs, one of which counted allocations. It was judged too complex for that gain.

### Allocating the log-stage ring buffers on first record: #2435

a193ac1c ([#2435]) makes `RangeMap` allocate its slots on its first record. Against its parent
8de65a99, measured for 10 s after 5 s of settling:

| Case                                        | 8de65a99 | a193ac1c |
|---------------------------------------------|---------:|---------:|
| 1,000 leaders, heap                         |  354 MiB |  214 MiB |
| 1,000 leaders, memory, median of three runs |  227 MiB |  212 MiB |
| 1,000 whole groups, heap                    |  964 MiB |  542 MiB |
| 1,000 whole groups, memory                  |  590 MiB |  542 MiB |
| 5,000 leaders, memory                       |  999 MiB |  918 MiB |

### Starting the engine and responder queues empty: #2440

b52ee511 ([#2440]) starts the engine's command queue, its four pending-respond queues and the
client responder queue of `RaftCore` without a buffer, instead of with 64 slots each. Against
the code before it, measured for 10 s after 5 s of settling:

| Case                                        | Before #2440 | b52ee511 |
|---------------------------------------------|-------------:|---------:|
| 1,000 leaders, heap                         |      214 MiB |  174 MiB |
| 1,000 leaders, memory, median of three runs |      215 MiB |  197 MiB |
| 1,000 whole groups, heap                    |      542 MiB |  423 MiB |
| 1,000 whole groups, memory                  |      548 MiB |  473 MiB |
| 5,000 leaders, memory                       |      931 MiB |  837 MiB |

Other jobs raised the load average from 14 to 55 during these runs, so they do not compare CPU.

### No histograms without the `runtime-stats` feature: #2487

d698b3a5 ([#2487]) gives the histograms of `RuntimeStats` a zero-sized type that records nothing
when the `runtime-stats` feature is off, since no API returns `RuntimeStats` then. A default
build then creates no base2histogram `Histogram`, so the process no longer builds its tables.
Against its parent ca32e84d, with `--count-allocs`, the medians of three runs of each build:

| Case                       | ca32e84d | d698b3a5 |
|----------------------------|---------:|---------:|
| 1,000 leaders, heap        |  171 MiB |  111 MiB |
| 1,000 leaders, memory      |  194 MiB |  132 MiB |
| 1,000 whole groups, heap   |  416 MiB |  287 MiB |
| 1,000 whole groups, memory |  467 MiB |  336 MiB |

The runs of each build gave the same heap. Per group, the heap fell by 62 KiB with stub
followers and by 132 KiB with whole groups, against the 59.7 and 126.7 KiB that the profile
predicts. Allocations and AppendEntries requests per second did not change. The load average
fell from 54 to 13 during the runs, so they do not compare CPU.

[#2304]: https://github.com/databendlabs/openraft/issues/2304
[#2343]: https://github.com/databendlabs/openraft/pull/2343
[#2378]: https://github.com/databendlabs/openraft/pull/2378
[#2400]: https://github.com/databendlabs/openraft/pull/2400
[#2407]: https://github.com/databendlabs/openraft/pull/2407
[#2412]: https://github.com/databendlabs/openraft/pull/2412
[#2413]: https://github.com/databendlabs/openraft/pull/2413
[#2419]: https://github.com/databendlabs/openraft/pull/2419
[#2427]: https://github.com/databendlabs/openraft/pull/2427
[#2434]: https://github.com/databendlabs/openraft/pull/2434
[#2435]: https://github.com/databendlabs/openraft/pull/2435
[#2440]: https://github.com/databendlabs/openraft/pull/2440
[#2454]: https://github.com/databendlabs/openraft/issues/2454
[#2456]: https://github.com/databendlabs/openraft/issues/2456
[#2487]: https://github.com/databendlabs/openraft/pull/2487
[#2488]: https://github.com/databendlabs/openraft/issues/2488
[rust-lang/rust#62958]: https://github.com/rust-lang/rust/issues/62958
