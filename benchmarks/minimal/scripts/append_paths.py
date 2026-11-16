#!/usr/bin/env python3
"""Run `bench` through both replication paths and print the README table.

Builds `bench` in release mode, then runs every combination of replication
path, network delay and replication read delay `--runs` times. Each round
runs every combination once, so a drift in machine load affects them alike.
Prints one markdown row per combination with the median of the runs. The
defaults reproduce the "Replication paths" section of
`benchmarks/minimal/README.md`.

The pipelined path passes the leader's request stream to the follower's
`Raft::stream_append()`; the sequential path (`--sequential-append`) sends
one AppendEntries request per round trip, like `stream_append_sequential()`.

Usage:
    python3 benchmarks/minimal/scripts/append_paths.py [--runs N] [--raw FILE]
"""

import argparse
import json
import statistics
import subprocess
import sys
from pathlib import Path

BENCH_DIR = Path(__file__).resolve().parent.parent
BENCH = BENCH_DIR / "target" / "release" / "bench"

PATHS = ["pipelined", "sequential"]

# (one-way network delay µs, replication read delay µs).
DELAYS = [(0, 0), (500, 0), (0, 500), (500, 500)]

CLIENTS = 256
OPERATIONS = "30k"

COLUMNS = [
    ("path", "path"),
    ("network µs", "network_us"),
    ("repl. read µs", "read_us"),
    ("writes/s", "ops_per_s"),
    ("p50 µs", "p50_us"),
    ("p99 µs", "p99_us"),
]


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--runs", type=int, default=5, help="runs per combination [default: 5]")
    parser.add_argument("--raw", type=Path, help="also write every run as one JSON line to this file")
    args = parser.parse_args()

    build = ["cargo", "build", "--release", "--bin", "bench"]
    subprocess.run(build, cwd=BENCH_DIR, check=True)

    raw = args.raw.open("w") if args.raw else None
    combinations = [(path, network_us, read_us) for network_us, read_us in DELAYS for path in PATHS]
    runs = {combination: [] for combination in combinations}
    for i in range(args.runs):
        for path, network_us, read_us in combinations:
            label = f"run={i + 1}/{args.runs} path={path} network={network_us}us read={read_us}us"
            print(label, file=sys.stderr, flush=True)
            run = run_bench(path, network_us, read_us)
            runs[(path, network_us, read_us)].append(run)
            if raw:
                print(json.dumps(run), file=raw, flush=True)

    print_table([median_row(runs[combination]) for combination in combinations])


def run_bench(path, network_us, read_us):
    """Run `bench` once and return its measurements."""
    command = [
        str(BENCH), "--latency", "-c", str(CLIENTS), "-n", OPERATIONS,
        "--network-delay-us", str(network_us), "--replication-read-delay-us", str(read_us),
    ]  # fmt: skip
    if path == "sequential":
        command.append("--sequential-append")
    result = subprocess.run(command, capture_output=True, text=True, check=True)
    report = json.loads(result.stdout.strip().splitlines()[-1])

    return {
        "path": path,
        "network_us": network_us,
        "read_us": read_us,
        "ops_per_s": round(report["operations_per_second"]),
        "p50_us": report["request_p50_ns"] // 1000,
        "p99_us": report["request_p99_ns"] // 1000,
    }


def median_row(runs):
    """Combine the runs of one combination into a table row of medians."""
    row = {"path": runs[0]["path"]}
    for key in runs[0]:
        if key != "path":
            row[key] = statistics.median(run[key] for run in runs)
    return row


def print_table(rows):
    cells = [[format_cell(key, row[key]) for _, key in COLUMNS] for row in rows]
    headers = [header for header, _ in COLUMNS]
    widths = [max(len(header), *(len(line[i]) for line in cells)) for i, header in enumerate(headers)]
    aligns = [":--" if key == "path" else "--:" for _, key in COLUMNS]
    print("| " + " | ".join(header.ljust(width) for header, width in zip(headers, widths)) + " |")
    print("| " + " | ".join(align.ljust(width) for align, width in zip(aligns, widths)) + " |")
    for line in cells:
        padded = [cell.ljust(w) if key == "path" else cell.rjust(w) for cell, w, (_, key) in zip(line, widths, COLUMNS)]
        print("| " + " | ".join(padded) + " |")


def format_cell(key, value):
    if key == "path":
        return value
    return f"{round(value):,}"


if __name__ == "__main__":
    main()
