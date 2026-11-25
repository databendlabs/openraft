# Results of grpc-groups

These measurements are the reference for the scenarios that [#2515] adds to grpc-groups and for
the heartbeat batching of [#2516], follow-ups of [#2304] that [#2454] tracks. The
[README](README.md) describes the scenarios, the phases and the fields.

All runs used one Apple M3 Max (16 cores, 128 GiB) with macOS 14.5, the machine of the
idle-groups [results](../idle-groups/results.md). They were release builds with the toolchain of
`rust-toolchain`, nightly-2026-03-01, with tokio 1.54.1 and tonic 0.14.9, and ran with the
default options and `Config`: 3 node processes with 4 worker threads each, and 1,000 groups.
Other work ran on the machine. Sampled every 30 s, its 1-minute load average was 7.5 to 45 during
the runs.

## Baseline at 51592651

[`measure_change.py`](../idle-groups/scripts/measure_change.py) built the benchmark at 51592651
twice, the second time with a comment appended to `src/main.rs`, and ran 5 rounds of `steady`,
then 5 of `crash`, with the two builds in turns. Each cell holds the median of the 10 runs of a
phase, with the smallest and the largest run in parentheses.

| Field               |  `measure` of `steady` |  `baseline` of `crash` |     `degraded` of `crash` |
|---------------------|-----------------------:|-----------------------:|--------------------------:|
| `cpu%`              |          243 (234–278) |          243 (204–260) |             197 (158–228) |
| `memory_mib`        |          465 (456–509) |          445 (432–558) |             325 (316–377) |
| `tasks/group`       |       25.0 (25.0–25.5) |       25.0 (25.0–26.3) |          18.4 (18.4–18.7) |
| `appends/s`         | 39,084 (38,503–39,130) | 39,127 (38,358–39,187) |    39,203 (38,046–39,450) |
| `hb_rtt_p50_us`     |    2,529 (1,700–2,963) |    2,544 (1,720–3,217) |       2,585 (1,694–3,313) |
| `hb_rtt_p99_us`     |  10,712 (6,633–35,144) |   7,211 (6,112–32,735) |     14,074 (6,778–40,025) |
| `hb_rtt_max_us`     | 61,015 (20,030–92,282) |  25,669 (9,593–77,606) |   26,006 (14,092–158,851) |
| `hb_failed`         |          106 (0–2,511) |            0 (0–2,563) | 195,322 (193,658–196,134) |
| `ack_age_p50_ms`    |       29.2 (28.3–31.1) |       29.3 (27.7–32.6) |          29.8 (28.3–34.4) |
| `ack_age_p99_ms`    |      55.3 (54.2–102.6) |      54.8 (53.3–104.8) |         57.4 (54.4–177.1) |
| `ack_age_max_ms`    |     115.1 (62.1–177.5) |      61.7 (57.1–176.9) |         68.8 (57.9–278.8) |
| `quorum_age_max_ms` |      95.1 (57.0–177.5) |      60.3 (56.0–176.9) |         68.8 (57.9–278.8) |

- `rpcs/s` equals `appends/s` in every window, because the router sends each request in its own
  gRPC call, and `votes/s` and `elections` are 0: no group changed its leader outside the crash.
- Before the crash, the leaders send 39,100 AppendEntries requests a second, close to the 40,000
  of a heartbeat every 50 ms from each of 1,000 leaders to each of its 2 followers.
- After the crash, the survivors send as many requests, and half of them fail: about 19,500 a
  second go to node 1. Replication backs off from an unreachable target, but `HeartbeatWorker`
  sends every heartbeat that is due and only logs its error.
- CPU falls with the crash, from 243% for 3 nodes to 197% for 2, but rises from 81% to 98% per
  node. Each survivor then leads its own 333 groups and its share of the 334 that node 1 led.
- In 13 of the 20 windows before the crash, the nodes ran exactly 25,033 tasks: 25 per group, 11
  for the leader member and 7 for each follower as in idle-groups, and 33 others. In 6 of the 10
  `degraded` windows they ran 18,350: 18 per group for its 2 remaining members, 16 others, and
  334 tasks of the benchmark itself. Each survivor watches each group that node 1 led until it
  takes the group over or 40 s pass, so when `degraded` ends, the survivor that did not take a
  group over still watches it. With 300 groups, a run with 5 s windows ended `degraded` with
  5,516 tasks, and one with 45 s windows with 5,416. The driver has since ended the watchers with
  `failover`: a later `crash` run at 1,000 groups ended `degraded` with 18,016 tasks.
- Heartbeats failed in 8 of the 20 windows before the crash, at most 0.67% of the requests of a
  window. Each of those windows also had a reply slower than 50 ms, the `heartbeat_interval`
  after which openraft drops a heartbeat; the other 12 had no reply slower than 46 ms.
- The acknowledgement ages are 29 ms at p50 and 55 to 57 ms at p99, about one heartbeat interval
  plus a round trip, but their maximum varies from run to run. `quorum_age_max_ms` exceeded
  150 ms in 6 of the 30 windows, and reached 279 ms in one `degraded` window, 21 ms short of the
  300 ms `leader_lease` at which a leader refuses writes.

The `failover` phases of the same runs, as the median of the 10 runs, with the smallest and the
largest run in parentheses:

| Percentile |        `elect_*_ms` |        `ready_*_ms` |
|------------|--------------------:|--------------------:|
| p50        | 472.8 (452.7–498.4) | 473.2 (452.9–512.0) |
| p90        | 535.3 (517.0–542.1) | 535.7 (517.7–544.4) |
| p99        | 581.7 (562.8–722.5) | 581.8 (562.9–722.9) |
| max        | 674.8 (577.6–763.6) | 675.0 (577.7–763.7) |

- In every run, each of the 334 groups that node 1 led got a new leader that committed an entry
  of its term, and no other group changed its leader: `unrecovered` and `moved` are 0.
- At the median, a group elects its new leader 473 ms after the kill, and at p99 582 ms after it,
  inside the 400 to 600 ms in which the first survivor of a group campaigns. In 9 of the 10
  runs, `ready_p50_ms` exceeds `elect_p50_ms` by less than 1 ms.
- A group retries when its first term elects no leader, as when both survivors campaign in the
  same term. 9 runs retried 0 to 4 groups, and one retried 20. The runs with a retried group had
  their slowest election at 630 to 764 ms, and the 2 runs without one at 578 and 583 ms.

## Comparing two builds

The two builds compiled the same code, so their medians differ only by chance. A comparison of 5
rounds shows a change only if the change moves a median by more than that. The difference
between the medians of the two builds, as a share of the median of the first build:

| Field               | `measure` of `steady` | `baseline` of `crash` | `degraded` of `crash` |
|---------------------|----------------------:|----------------------:|----------------------:|
| `cpu%`              |                  3.1% |                  3.0% |                  8.3% |
| `memory_mib`        |                  1.5% |                  1.3% |                    0% |
| `appends/s`         |                  0.2% |                  0.2% |                  0.1% |
| `hb_rtt_p50_us`     |                  8.3% |                   16% |                   12% |
| `hb_rtt_p99_us`     |                  5.5% |                   38% |                   86% |
| `hb_rtt_max_us`     |                  9.0% |                   60% |                   18% |
| `ack_age_p50_ms`    |                  2.4% |                  0.7% |                  2.0% |
| `ack_age_p99_ms`    |                  2.0% |                  0.2% |                  5.9% |
| `ack_age_max_ms`    |                   34% |                   24% |                  1.7% |
| `quorum_age_max_ms` |                   30% |                   22% |                  1.7% |

- Rates, memory and the ages at p50 and p99 differ by at most 2.4% before the crash and 5.9%
  after it, and CPU by 3.1% and 8.3%.
- The round trip differs by 8% to 16% at p50 and by up to 86% at p99 and at its maximum, and the
  maximum ages by up to 34%. Comparing these needs more rounds, or a far larger change.
- The failover percentiles differ by at most 0.5% at p50 and p90, 2.7% at p99 and 1.5% at the
  maximum.
- Counts of rare events differ the most: the medians of `hb_failed` in `steady` are 881 and 4,
  and those of `retried` 2 and 1.

To repeat these runs, from the root of a checkout:

```sh
printf '\n// A/A run: this comment is the only change between the two builds.\n' \
    >> benchmarks/grpc-groups/src/main.rs
benchmarks/idle-groups/scripts/measure_change.py baseline benchmarks/grpc-groups/src/main.rs \
    --bench grpc-groups --case '5 steady' --case '5 crash'
git checkout -- benchmarks/grpc-groups/src/main.rs
```

[#2304]: https://github.com/databendlabs/openraft/issues/2304
[#2454]: https://github.com/databendlabs/openraft/issues/2454
[#2515]: https://github.com/databendlabs/openraft/issues/2515
[#2516]: https://github.com/databendlabs/openraft/issues/2516
