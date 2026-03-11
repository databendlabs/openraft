# 10 - Add long-running soak, stress, and regression-detection jobs

## Objective

Detect flakes, hangs, regressions, and resource growth that only show up over extended runs.

## Implementation steps

- Define a small number of long-running workloads that combine writes, membership changes, snapshots, restarts, and injected faults over hours rather than minutes.
- Add metrics collection for throughput, latency, memory growth, open file count, task count, and retry or error counts while the workloads run.
- Set concrete thresholds that make a run fail on deadlock, hang, unbounded memory growth, or major performance regression.
- Persist historical run summaries as CI artifacts first using the artifact layout defined in `README.md` in this directory, with stable filenames for metrics, thresholds, and failure summaries.
- If long-term storage outside CI is later needed, document that as a separate follow-up task instead of leaving it implicit.

## Deliverables

- one soak job definition;
- one metrics and threshold specification;
- one historical summary artifact per run.
