# Idle groups

This benchmark measures what many Raft groups cost one process while they have nothing to do:
tasks, memory and CPU. It follows the setup of
[#2304](https://github.com/databendlabs/openraft/issues/2304):

- One process holds the leader member of every group. The groups use `openraft-memstore`, an
  `openraft-multi` `GroupRouter`, and the default `Config`.
- By default the two followers of each group are stubs in the router. They grant every vote and
  acknowledge every AppendEntries request, so the numbers cover the leader members only. With
  `--followers local` the followers are real members in the same process, and the router
  delivers each request to its target member, so the numbers cover whole groups.
- Once every group has elected its leader, the benchmark switches the timers that `--mode`
  names, waits `--settle-secs`, and measures for `--measure-secs`. Nothing is written, unless
  `--burst-writes` asks for a burst of writes before the settle period; see
  [Write burst](#write-burst).

## Running

```sh
cargo run --release --manifest-path benchmarks/idle-groups/Cargo.toml -- --groups 1000 --mode no-tick
```

| Option           | Default   | Meaning                                                   |
|------------------|-----------|-----------------------------------------------------------|
| `--groups`       | 1000      | The number of groups.                                     |
| `--followers`    | `stub`    | What stands in for the followers: `stub` or `local`.      |
| `--mode`         | `default` | Which timers stay on: see the modes below.                |
| `--count-allocs` | off       | Count heap allocations, for `heap_mib` and `allocs/s`.    |
| `--burst-writes` | 0         | Writes to each group in a burst before the settle period. |
| `--burst-depth`  | 16        | The writes of a burst in flight at once per group.        |
| `--settle-secs`  | 10        | Seconds to wait after switching the timers.               |
| `--measure-secs` | 30        | Seconds to measure for.                                   |
| `--workers`      | 4         | The worker threads of the tokio runtime.                  |

| Mode                 | Switches off, through `Raft::runtime_config()` |
|----------------------|------------------------------------------------|
| `default`            | nothing                                        |
| `no-elect`           | elections: `elect(false)`                      |
| `no-heartbeat`       | heartbeats: `heartbeat(false)`                 |
| `no-heartbeat-elect` | heartbeats and elections                       |
| `no-tick`            | ticks: `tick(false)`                           |

What `tick(false)` stops depends on the version. Up to 0.10.0 it stops the ticker, which drives
both heartbeats and elections. Since #2378 it pauses only a leader's heartbeat timer, so on
`main` the `no-tick` mode behaves like `no-heartbeat`.

The modes switch every member. With `--followers local`, a mode that turns heartbeats off and
keeps elections on makes the followers campaign, so the groups keep electing new leaders.

It prints one line of tab-separated fields:

- `tasks`: the tasks alive in the tokio runtime at the end.
- `memory_mib`: the memory the process holds at the end. On macOS this is the physical
  footprint, which counts compressed pages. RSS leaves those out, and macOS compresses the pages
  that idle groups leave untouched. On Linux it is `VmRSS` plus `VmSwap`.
- `heap_mib`: with `--count-allocs`, the heap memory that the process requested and has not
  freed at the end, as a wrapper around the system allocator counts it. It counts the full size
  of each allocation, but the OS backs only the pages that are written to, so it can exceed
  `memory_mib`.
- `cpu%`: the CPU time during the measurement, as a share of one core.
- `allocs/s`: with `--count-allocs`, the heap allocations per second during the measurement,
  including reallocations.
- `appends/s`: the AppendEntries requests that the followers received per second.
- `elections`: how far the highest term of each group grew during the measurement, summed over
  the groups. It stays 0 while every group keeps its leader.

Built with `--features runtime-stats`, which turns on the `runtime-stats` feature of openraft, it
also reads `Raft::runtime_stats()` of every member before and after the measurement, and adds
these rates, summed over the members. Without the feature they are `n/a`.

- `fires/s`: the timer deadlines reached per second: the heartbeats, elections and pending-read
  checks that `RaftCore` was notified of.
- `rearms/s`: the wakes per second that `RaftCore` gave a timer's task to re-arm it, by setting
  a deadline earlier than the stored one, or any deadline when the stored one has passed or is
  unset. Every heartbeat that fires is followed by one. A timer that expires at a deadline that
  has since moved later re-arms without either count.
- `metrics/s`: the `RaftMetrics` sends per second, one per metrics report.
- `data_metrics/s` and `server_metrics/s`: the `RaftDataMetrics` and `RaftServerMetrics` sends
  per second. These are sent only when one of their fields changed.

The two reads of the stats fall outside the CPU window. Compare `cpu%` only between builds that
both have the feature or both do not, because openraft's `runtime-stats` feature also times each
turn of its core loop.

```sh
cargo run --release --manifest-path benchmarks/idle-groups/Cargo.toml --features runtime-stats -- --groups 1000
```

Counting allocations costs CPU: at 5,000 groups with `--followers local`, which make 6.1 million
allocations a second, it adds 7% to 11% to `cpu%`. Compare `cpu%` only between runs that both
count or both do not. Without `--count-allocs` the wrapper reads one flag per allocation, and
`cpu%` stays within the 3% that runs differ by.

To measure a release, point the three openraft dependencies in `Cargo.toml` at it, for example
`openraft = "=0.10.0"`, `openraft-memstore = "=0.10.0"` and `openraft-multi = "=0.10.0"`.

## Write burst

With `--burst-writes N`, the benchmark writes `N` entries to the leader of every group right after
switching the timers, at most `--burst-depth` of them in flight per group. It starts once a quorum
has acknowledged every leader, since a leader rejects writes until then, and the settle period
starts when the last write responds. The measurement window then shows the memory that the burst
left behind, against an idle run with `--burst-writes 0`. That memory includes the entries the
burst wrote: `openraft-memstore` keeps every log entry until a snapshot purges it, and the default
`Config` builds no snapshot below 5,000 entries.

A leader that no quorum has acknowledged recently rejects a write with
`ForwardReason::LeaseExpired`, and the burst writes it again after 10 ms. Any other rejected write
stops the run, as when another member of a group takes over its leadership: to keep the leaders
of whole groups, run `--mode no-elect`.

It adds these fields, `n/a` without a burst:

- `burst_writes`: the writes, summed over the groups.
- `burst_retries`: the writes that a leader rejected for an expired lease and the burst retried.
- `burst_writes/s`: the writes per second, from the first write to the last response.
- `burst_p50_us`, `burst_p99_us` and `burst_max_us`: the latency of a write, from its first
  attempt to its response, in microseconds. The percentiles come from a log-scale histogram,
  within 1.6% of the true value; the largest is exact.

```sh
cargo run --release --manifest-path benchmarks/idle-groups/Cargo.toml -- \
    --groups 1000 --followers local --burst-writes 1000 --burst-depth 16 --count-allocs
```

## Measuring a change

[`scripts/measure_change.py`](scripts/measure_change.py) compares a change in a checkout with the
code before it. It builds the benchmark twice in that checkout: first with the files of the
change set to their versions at `--base`, `HEAD` by default, and then with the change. It then
runs each `--case` with the two builds in turns, so that both see the same machine load.

```sh
benchmarks/idle-groups/scripts/measure_change.py my-change $(git diff --name-only) \
    --case '3 1000 stub default' \
    --case '1 1000 local default --count-allocs'
```

A case is `RUNS GROUPS FOLLOWERS MODE [EXTRA ARGS]` and runs `RUNS` times with each build. The
script writes one line per run to `results/LABEL-<time>.tsv` and keeps both binaries in `bin/`,
under `benchmarks/idle-groups/target/measure-change/`, or under `--out-dir` if it is given. To
measure a commit instead of uncommitted changes, compare it with its parent: pass
`--base HEAD~1` and the files of `git diff --name-only HEAD~1`.

At the end, the script prints the median of each field for each build, as a Markdown table per
case. With `--bench grpc-groups`, it builds and runs
[`benchmarks/grpc-groups`](../grpc-groups/README.md#measuring-a-change) instead, whose cases are
`RUNS SCENARIO [EXTRA ARGS]`, and prints a table per phase of each case.

The second build recompiles only what the files of the change affect. Do not edit those files
while the script runs. Other load on the machine moves `cpu%`, so check the load average before
and after a comparison of CPU.

## Results

These compare 0.10.0 with `main` at 1329ad82. They ran on an Apple M3 Max with macOS 14.5 and
rustc 1.101.0-nightly, as release builds with the system allocator, one run per row. Every
member runs 10 tasks on 0.10.0 and 11 at 1329ad82.

| Groups | Mode                 | 0.10.0 CPU | 0.10.0 memory | 1329ad82 CPU | 1329ad82 memory |
|-------:|----------------------|-----------:|--------------:|-------------:|----------------:|
|  1,000 | `default`            |        83% |       332 MiB |          33% |         235 MiB |
|  1,000 | `no-heartbeat-elect` |        67% |       212 MiB |           0% |         222 MiB |
|  1,000 | `no-tick`            |         0% |       200 MiB |           0% |         223 MiB |
|  5,000 | `default`            |       256% |     1,450 MiB |         167% |       1,038 MiB |
|  5,000 | `no-heartbeat-elect` |       255% |       896 MiB |           0% |         992 MiB |
|  5,000 | `no-tick`            |         0% |       852 MiB |           0% |         991 MiB |

On 0.10.0 the ticks cost most of the CPU: with heartbeats and elections off, 1,000 groups still
use 67% of a core and 5,000 groups 255%. At 1329ad82, idle groups with heartbeats and elections
off use no CPU, and with defaults the CPU goes to the heartbeats: 36,685 AppendEntries requests
a second at 1,000 groups.

[results.md](results.md) has the measurements of the changes that followed, from retiring the
ticker in #2378 to boxing the rarely sent channel messages in #2488, and of the harness additions
of #2455 and #2459.
