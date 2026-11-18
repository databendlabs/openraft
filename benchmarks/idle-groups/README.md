# Idle groups

This benchmark measures what many Raft groups cost one process while they have nothing to do:
tasks, memory and CPU. It follows the setup of
[#2304](https://github.com/databendlabs/openraft/issues/2304):

- One process holds the leader member of every group. The groups use `openraft-memstore`, an
  `openraft-multi` `GroupRouter`, and the default `Config`.
- The two followers of each group are stubs in the router. They grant every vote and acknowledge
  every AppendEntries request, so the numbers cover the leader side only.
- Once every group has elected its leader, the benchmark switches the timers that `--mode`
  names, waits `--settle-secs`, and measures for `--measure-secs`. Nothing is written.

## Running

```sh
cargo run --release --manifest-path benchmarks/idle-groups/Cargo.toml -- --groups 1000 --mode no-tick
```

| Option           | Default   | Meaning                                                                                                                   |
|------------------|-----------|---------------------------------------------------------------------------------------------------------------------------|
| `--groups`       | 1000      | The number of groups.                                                                                                     |
| `--mode`         | `default` | `default` keeps every timer on, `no-heartbeat-elect` turns heartbeats and elections off, and `no-tick` turns ticks off. |
| `--settle-secs`  | 10        | Seconds to wait after switching the timers.                                                                               |
| `--measure-secs` | 30        | Seconds to measure for.                                                                                                   |
| `--workers`      | 4         | The worker threads of the tokio runtime.                                                                                  |

It prints one line of tab-separated fields:

- `tasks`: the tasks alive in the tokio runtime at the end.
- `memory_mib`: the memory the process holds at the end. On macOS this is the physical
  footprint, which counts compressed pages. RSS leaves those out, and macOS compresses the pages
  that idle groups leave untouched. On Linux it is `VmRSS` plus `VmSwap`.
- `cpu%`: the CPU time during the measurement, as a share of one core.
- `appends/s`: the AppendEntries requests that the stub followers received per second.

To measure a release, point the three openraft dependencies in `Cargo.toml` at it, for example
`openraft = "=0.10.0"`, `openraft-memstore = "=0.10.0"` and `openraft-multi = "=0.10.0"`.

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
