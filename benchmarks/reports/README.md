# Benchmark reports

Each report lives in its own directory, named `<date>-<subject>`. A directory holds:

- `report-full.md`: setup, method, all results, analysis, limits, and how to reproduce.
- `results/`: generated tables, charts, and any harness patches the runs needed. Raw
  per-run output is not committed.

| date       | report                                                      | summary |
| :--        | :--                                                         | :--     |
| 2026-11-17 | [v0.10.0 vs main](2026-11-17-v0.10.0-vs-main/report-full.md) | `benchmarks/minimal` writes with simulated append, fsync, network, and apply delays. No change with a fast state machine; 2.2× to 4.8× more writes/s when apply is the bottleneck. |
| 2026-11-25 | [Apply lag on main](2026-11-25-apply-lag/report-full.md)     | `benchmarks/minimal` leader stage latencies and apply backlog, paced and saturated, with slow `apply()` calls. Commit does not wait for apply; commit to apply takes 1.4 to 1.9 calls of 500 µs or more below saturation, and grows with load for a cost per entry. At saturation, 256 waiting clients settle into one group or two groups of writes per call; with two groups each write waits for most of an extra call. |
| 2026-11-26 | [Follower catch-up on main](2026-11-26-follower-catch-up/report-full.md) | `benchmarks/minimal` with one follower cut off for 1 or 5 s. The replication backoff, 200 ms by default, sets when its log reaches the leader's commit index: about 605 to 640 ms after the reconnection. Apply then takes 0 to 120 ms with a cost per call, in calls of 1,024 entries, and 540 to 790 ms with 10 µs per entry. The leader's writes/s and p99 do not change while the follower catches up. |
| 2026-11-27 | [Apply lag on real disk](2026-11-27-apply-lag-on-real-disk/report-full.md) | `benchmarks/durable` WAL nodes with real `F_FULLFSYNC`, driven by `process_profile.py`, with `apply()` delays of 0, 2 and 10 ms. The disk sets commit at about 20 ms per append; apply lags commit by 1.2 calls of 2 ms and 1.8 calls of 10 ms, so with 10 ms calls apply is 44 to 47% of a write's time on the leader. |
