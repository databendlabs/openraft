# Benchmark reports

Each report lives in its own directory, named `<date>-<subject>`. A directory holds:

- `report-full.md`: setup, method, all results, analysis, limits, and how to reproduce.
- `results/`: generated tables, charts, and any harness patches the runs needed. Raw
  per-run output is not committed.

| date       | report                                                      | summary |
| :--        | :--                                                         | :--     |
| 2026-11-17 | [v0.10.0 vs main](2026-11-17-v0.10.0-vs-main/report-full.md) | `benchmarks/minimal` writes with simulated append, fsync, network, and apply delays. No change with a fast state machine; 2.2× to 4.8× more writes/s when apply is the bottleneck. |
| 2026-11-25 | [Apply lag on main](2026-11-25-apply-lag/report-full.md)     | `benchmarks/minimal` leader stage latencies and apply backlog, paced and saturated, with slow `apply()` calls. Commit does not wait for apply; commit to apply takes 1.4 to 1.9 calls of 500 µs or more below saturation, up to 2.3 at saturation, and grows with load for a cost per entry. |
