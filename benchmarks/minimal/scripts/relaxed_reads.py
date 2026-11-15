#!/usr/bin/env python3
"""Compare core, local, and relaxed lease reads including a coherent state-machine query.

Build once without runtime-stats, rotate run order, and report median latency and
throughput. --raw FILE preserves every JSON report. No read failures are allowed.

--grid follower instead runs standard reads on follower 1, with and without
enable_relaxed_read, to measure what the follower apply bound costs.
"""

import argparse
import json
import platform
import statistics
import subprocess
import sys
from pathlib import Path

BENCH_DIR = Path(__file__).resolve().parent.parent
BENCH = BENCH_DIR / "target" / "release" / "bench"
MODES = [
    # (name, enable_local_lease_read, enable_relaxed_read, extra bench arguments)
    ("core", False, False, []),
    ("local", True, False, []),
    ("relaxed", True, True, ["--read-relaxed"]),
]
# Standard follower reads: the leader issues a cached LeaseRead linearizer, and follower 1 waits
# for it and runs the query. "bound" only enables the follower apply bound.
FOLLOWER_MODES = [
    ("follower", True, False, ["--read-on-follower", "1"]),
    ("follower+bound", True, True, ["--read-on-follower", "1"]),
]
SCENARIOS = [
    # (label, write clients, writes, read clients, reads, read delay us, I/O delay us)
    ("read-only/1", 0, 0, 1, 20000, 0, 0),
    ("read-only/64", 0, 0, 64, 200000, 0, 0),
    ("mixed/no-delay", 32, 20000, 64, 180000, 0, 0),
    ("mixed/log-read-500us", 32, 20000, 64, 180000, 500, 0),
    ("mixed/apply-io-500us", 32, 20000, 64, 180000, 0, 500),
]
# Followers trail the leader only while writes arrive.
FOLLOWER_SCENARIOS = [scenario for scenario in SCENARIOS if scenario[1]]
GRIDS = {"leader": (MODES, SCENARIOS), "follower": (FOLLOWER_MODES, FOLLOWER_SCENARIOS)}


def command(scenario, mode):
    label, writers, writes, readers, reads, read_delay, io_delay = scenario
    name, local, relaxed, extra = mode
    config = {
        "enable_local_lease_read": local,
        "enable_relaxed_read": relaxed,
        "enable_tick": False,
        "enable_heartbeat": False,
        "enable_elect": False,
        "snapshot_policy": "Never",
    }
    args = [
        str(BENCH), "--latency", "--members", "3", "--warmup-writes", "128",
        "--clients", str(writers), "--operations", str(writes),
        "--read-clients", str(readers), "--read-operations", str(reads),
        "--read-query", "--apply-read-delay-us", str(read_delay),
        "--apply-io-delay-us", str(io_delay), "--raft-config", json.dumps(config),
    ]
    if writers:
        args.extend(["--write-rate", "10000", "--read-rate", "90000"])
    args.extend(extra)
    return args


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--runs", type=int, default=3)
    parser.add_argument("--raw", type=Path)
    parser.add_argument("--skip-build", action="store_true")
    parser.add_argument("--grid", choices=GRIDS, default="leader")
    args = parser.parse_args()
    modes, scenarios = GRIDS[args.grid]
    if args.runs < 1:
        parser.error("--runs must be positive")
    if not args.skip_build:
        subprocess.run(
            ["cargo", "build", "--release", "--no-default-features", "--bin", "bench"],
            cwd=BENCH_DIR, check=True,
        )
    reports = {}
    raw = args.raw.open("w") if args.raw else None
    try:
        for scenario in scenarios:
            label = scenario[0]
            for run in range(args.runs):
                order = modes[run % len(modes):] + modes[:run % len(modes)]
                for mode in order:
                    name = mode[0]
                    print(f"{label} {name} run {run + 1}/{args.runs}", file=sys.stderr, flush=True)
                    cmd = command(scenario, mode)
                    result = subprocess.run(cmd, capture_output=True, text=True, timeout=420)
                    if result.returncode:
                        sys.stderr.write(result.stderr)
                        raise RuntimeError(f"benchmark exited {result.returncode}")
                    report = json.loads(result.stdout.strip().splitlines()[-1])
                    if report["read"]["failed"]["operations"]:
                        raise RuntimeError(f"read failures: {report['read']['errors']}")
                    expected_writes, expected_reads = scenario[2], scenario[4]
                    if report["write"]["operations"] != expected_writes:
                        raise RuntimeError("benchmark did not complete all writes")
                    if report["read"]["successful"]["operations"] != expected_reads:
                        raise RuntimeError("benchmark did not complete all reads")
                    reports.setdefault((label, name), []).append(report)
                    if raw:
                        record = {"scenario": label, "mode": name, "run": run + 1, "command": cmd, "report": report}
                        print(json.dumps(record), file=raw, flush=True)
    finally:
        if raw:
            raw.close()

    print(f"Machine: {platform.platform()}, {platform.machine()}; medians of {args.runs} runs.")
    print("All modes include the apply barrier and the same lock-protected state-machine counter query.")
    follower = args.grid == "follower"
    lag_header = " Lag p99 entries | Lag p99 µs |" if follower else ""
    print(f"| Workload | Mode | Reads/s | Read p50 µs | Read p99 µs | Writes/s |{lag_header}")
    print("| --- | --- | ---: | ---: | ---: | ---: |" + (" ---: | ---: |" if follower else ""))
    for scenario in scenarios:
        for name, *_ in modes:
            values = reports[(scenario[0], name)]
            reads = [r["read"]["successful"] for r in values]
            rate = statistics.median(r["operations_per_second"] for r in reads)
            p50 = statistics.median(r["latency"]["p50_ns"] for r in reads) / 1000
            p99 = statistics.median(r["latency"]["p99_ns"] for r in reads) / 1000
            writes = statistics.median(r["write"]["operations_per_second"] for r in values)
            row = f"| {scenario[0]} | {name} | {rate:,.0f} | {p50:.3f} | {p99:.3f} | {writes:,.0f} |"
            if follower:
                lags = [r["follower_lag"] for r in values]
                entries = statistics.median(lag["entries_behind"]["p99"] for lag in lags)
                micros = statistics.median(lag["time_behind_ns"]["p99"] for lag in lags) / 1000
                row += f" {entries:,.0f} | {micros:,.0f} |"
            print(row)


if __name__ == "__main__":
    main()
