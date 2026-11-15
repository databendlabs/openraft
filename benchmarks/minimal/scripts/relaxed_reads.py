#!/usr/bin/env python3
"""Compare core, local, and relaxed lease reads including a coherent state-machine query.

Build once without runtime-stats, rotate run order, and report median latency and
throughput. --raw FILE preserves every JSON report. No read failures are allowed.
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
MODES = [("core", False, False), ("local", True, False), ("relaxed", True, True)]
SCENARIOS = [
    # (label, write clients, writes, read clients, reads, read delay us, I/O delay us)
    ("read-only/1", 0, 0, 1, 20000, 0, 0),
    ("read-only/64", 0, 0, 64, 200000, 0, 0),
    ("mixed/no-delay", 32, 20000, 64, 180000, 0, 0),
    ("mixed/log-read-500us", 32, 20000, 64, 180000, 500, 0),
    ("mixed/apply-io-500us", 32, 20000, 64, 180000, 0, 500),
]


def command(scenario, mode):
    label, writers, writes, readers, reads, read_delay, io_delay = scenario
    name, local, relaxed = mode
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
    if relaxed:
        args.append("--read-relaxed")
    return args


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--runs", type=int, default=3)
    parser.add_argument("--raw", type=Path)
    parser.add_argument("--skip-build", action="store_true")
    args = parser.parse_args()
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
        for scenario in SCENARIOS:
            label = scenario[0]
            for run in range(args.runs):
                order = MODES[run % len(MODES):] + MODES[:run % len(MODES)]
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
    print("| Workload | Mode | Reads/s | Read p50 µs | Read p99 µs | Writes/s |")
    print("| --- | --- | ---: | ---: | ---: | ---: |")
    for scenario in SCENARIOS:
        for name, _, _ in MODES:
            values = reports[(scenario[0], name)]
            reads = [r["read"]["successful"] for r in values]
            rate = statistics.median(r["operations_per_second"] for r in reads)
            p50 = statistics.median(r["latency"]["p50_ns"] for r in reads) / 1000
            p99 = statistics.median(r["latency"]["p99_ns"] for r in reads) / 1000
            writes = statistics.median(r["write"]["operations_per_second"] for r in values)
            print(f"| {scenario[0]} | {name} | {rate:,.0f} | {p50:.3f} | {p99:.3f} | {writes:,.0f} |")


if __name__ == "__main__":
    main()
